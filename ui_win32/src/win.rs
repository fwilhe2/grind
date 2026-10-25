// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The window: its class, its procedure, and the loop that pumps it.
//!
//! **Windows only, and the only file in this crate that holds state.** Everything it decides is
//! decided somewhere portable — `sheet/geom.rs` where a cell is, `sheet/draw.rs` what it looks
//! like, `theme.rs` its colour — so this file translates Windows' messages into those answers,
//! plus the one piece of unsafe machinery the shell needs.
//!
//! ## The `GWLP_USERDATA` arrangement
//!
//! A window procedure is a C callback with an `HWND` and no context, so the state is `Box`ed, its
//! raw pointer handed to the window in `GWLP_USERDATA` and read back on every message. That is
//! safe on three facts a future edit can break:
//!
//! 1. The pointer is stored in `WM_NCCREATE`, from the caller's `CREATESTRUCTW`, before any
//!    message that reads it.
//! 2. It is taken back and dropped in `WM_NCDESTROY`, the *last* message a window gets, and the
//!    slot zeroed then.
//! 3. **A message is never dispatched re-entrantly while the borrow is live** (decision 7). Each
//!    handler takes the `&mut` for one message; `dialog.rs`'s modals run nested message loops, so
//!    a handler opening one borrows **on each side of the call and never across it** —
//!    [`sheet_rename`] is the shape: one [`with_sheet`] to read, the prompt, a *fresh*
//!    [`with_sheet`] to apply, each with the SAFETY comment saying which side it is on.
//!
//! ## Editing state
//!
//! * a **mode** ([`crate::sheet::state::Mode`]) — Ready, Enter or Edit — stored here because the
//!   next keystroke depends on it;
//! * a child `EDIT` holding the in-progress text — a widget holding a keystroke, never the
//!   document (decision 2): emptied into `App::enter` and hidden again;
//! * a **dirty flag**, which the core does not have, set by an [`grind_core::Observer`] posting
//!   [`WM_DOC_CHANGED`], so undo, redo and a recalculation mark the document modified with no
//!   handler remembering to (rule 3).

#![cfg(windows)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, ClientToScreen, EndPaint, HDC, InvalidateRect, OPAQUE, PAINTSTRUCT, ScreenToClient,
    SetBkColor, SetBkMode, SetTextColor, UpdateWindow,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForWindow, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CREATESTRUCTW, CS_DBLCLKS, CW_USEDEFAULT, CheckMenuItem, CreateMenu,
    CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow, DispatchMessageW,
    EN_CHANGE, EN_KILLFOCUS, ES_AUTOHSCROLL, GWLP_USERDATA, GetCursorPos, GetMessageW, GetParent,
    GetWindowLongPtrW, GetWindowTextLengthW, GetWindowTextW, HMENU, IDC_ARROW, IDC_SIZEALL,
    IDC_SIZENESW, IDC_SIZENS, IDC_SIZENWSE, IDC_SIZEWE, LoadCursorW, MF_BYCOMMAND, MF_CHECKED,
    MF_GRAYED, MF_POPUP, MF_SEPARATOR, MF_STRING, MF_UNCHECKED, MSG, MoveWindow, PostMessageW,
    PostQuitMessage, RegisterClassW, SB_BOTTOM, SB_HORZ, SB_LINEDOWN, SB_LINEUP, SB_PAGEDOWN,
    SB_PAGEUP, SB_THUMBPOSITION, SB_THUMBTRACK, SB_TOP, SB_VERT, SCROLLINFO, SCROLLINFO_MASK,
    SIF_PAGE, SIF_POS, SIF_RANGE, SPI_GETWHEELSCROLLLINES, SW_HIDE, SW_SHOW, SWP_FRAMECHANGED,
    SWP_NOACTIVATE, SWP_NOZORDER, SendMessageW, SetCursor, SetWindowLongPtrW, SetWindowPos,
    SetWindowTextW, ShowWindow, SystemParametersInfoW, TPM_RETURNCMD, TPM_RIGHTBUTTON,
    TrackPopupMenuEx, TranslateMessage, WHEEL_DELTA, WM_APP, WM_CHAR, WM_CLOSE, WM_COMMAND,
    WM_CONTEXTMENU, WM_CREATE, WM_CTLCOLOREDIT, WM_DESTROY, WM_DPICHANGED, WM_ERASEBKGND,
    WM_HSCROLL, WM_IME_STARTCOMPOSITION, WM_INITMENUPOPUP, WM_KEYDOWN, WM_KILLFOCUS,
    WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_NCCREATE,
    WM_NCDESTROY, WM_PAINT, WM_SETCURSOR, WM_SETFOCUS, WM_SETFONT, WM_SETTINGCHANGE, WM_SIZE,
    WM_VSCROLL, WNDCLASSW, WS_CHILD, WS_HSCROLL, WS_OVERLAPPEDWINDOW, WS_VSCROLL,
};
// Focus and mouse capture are Windows' input API rather than its window-management one, which
// is where its own metadata puts them.
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, ReleaseCapture, SetCapture, SetFocus, VK_CONTROL, VK_MENU, VK_SHIFT,
};
// The one corner of IME support this shell has: positioning the composition window the IME
// draws for itself. Everything else about composing — the candidate list, committing a result —
// stays with `DefWindowProcW`, which is why this is the only IME import.
use windows::Win32::UI::Input::Ime::{
    CFS_POINT, COMPOSITIONFORM, ImmGetContext, ImmReleaseContext, ImmSetCompositionWindow,
};
// The caret API — a system caret's whole vocabulary — and the text pane's is Win32's real
// object rather than a drawn rectangle since `place_system_caret`.
use windows::Win32::UI::WindowsAndMessaging::{
    CreateCaret, DestroyCaret, HideCaret, SetCaretPos, ShowCaret,
};
// `SetScrollInfo` lives in the Controls namespace in Windows' own metadata, which is where the
// scrollbar API has always been. It is *not* a Common Controls v6 class and needs no manifest —
// `doc/windows-shell.md`'s rejection of a v6 toolbar does not reach it.
use windows::Win32::UI::Controls::{EM_GETSEL, EM_REPLACESEL, EM_SETSEL, SetScrollInfo};
use windows::core::PCWSTR;

use crate::clipboard;
use crate::code;
use crate::dialog::{self, Answer, Com};
use crate::gdi::{self, BackBuffer, Brush, Dib, Font};
use crate::image;
use crate::menu::{self, Command, Item};
use crate::menubar;
use crate::metrics::{Faces, Fonts};
use crate::notice;
use crate::problems;
use crate::sheet::assist;
use crate::sheet::currency;
use crate::sheet::draw::{self, FormatStrip, Frame};
use crate::sheet::format;
use crate::sheet::geom::{Edge, GridGeom, Hit, MAX_COLS, MAX_ROWS, Rect, Sizes, mm_to_px, scale};
use crate::sheet::keymap::{self, Dir, Selection};
use crate::sheet::measure;
use crate::sheet::state::{self, Outcome, Seed};
use crate::sheet::status;
use crate::surrogate;
use crate::text;
use crate::text::geom::{Flow, Page, StripHit};
use crate::theme::{self, Mode, Theme};
use crate::welcome;
use grind_core::DocumentKind;
use grind_sheet::clip;
use grind_sheet::{App, Filter, Pos, RecalcMode, TableOptions, a1, csv, find};
use grind_text::caret::START;
use grind_text::{Caret, Layout, markdown};
use windows::Win32::UI::WindowsAndMessaging::{
    HTMENU, SC_KEYMENU, WM_NCACTIVATE, WM_NCCALCSIZE, WM_NCHITTEST, WM_NCLBUTTONDOWN,
    WM_NCMOUSELEAVE, WM_NCMOUSEMOVE, WM_NCPAINT, WM_SYSCOMMAND,
};

/// The display name, which is not the file name (`doc/windows-shell.md`, decision 1).
const APP_NAME: &str = "Grind";
const CLASS_NAME: &str = "GrindWindowClass";

/// The face everything in this window is drawn in, **asked for rather than assumed**
/// (`gdi::ui_face`): *Segoe UI Variable Text* on Windows 11, Segoe UI on Windows 10, probed with
/// `EnumFontFamiliesExW` since GDI substitutes rather than fails. Under Wine neither is usually
/// installed, and GDI's substitute is what every screenshot there shows.
fn face() -> &'static str {
    gdi::ui_face()
}

/// The two child `EDIT`s, one id each — `WM_COMMAND`'s low word is how a notification says
/// which child it came from. Both are below [`menu::FIRST_ID`], which is what keeps a control's
/// notification and a menu item's click apart in the one message Win32 uses for both.
const ID_NAME_BOX: usize = 1;
const ID_EDITOR: usize = 2;

/// The size prose is set at, in pixels at 100% — the ramp's own, and larger than a cell's for
/// the reason `theme::text::PROSE` gives. Headings scale off this (`metrics.rs`).
const TEXT_PX: i32 = theme::text::PROSE as i32;

/// "A key went to one of our child controls", sent by the message loop. Enter, Escape, Tab and the
/// arrows never reach a child `EDIT`'s parent — the control swallows them — so the pump in [`run`]
/// relays them, the standard answer for a window with no dialog manager, and cheaper than a
/// subclass (a second procedure and lifetime, and `SetWindowSubclass` lives in `comctl32`, which
/// this binary does not import). The reply makes it a question: non-zero for a key the shell
/// claimed, zero for one the control should handle.
const WM_CHILD_KEY: u32 = WM_APP + 1;

/// "The document changed", posted by [`Changed`].
const WM_DOC_CHANGED: u32 = WM_APP + 2;

/// The frame `--render-to` draws, in pixels at 96 dpi — fixed, since a size that could differ
/// between two runs would show up as a drawing change.
const RENDER_W: i32 = 1280;
const RENDER_H: i32 = 800;

/// Everything the window owns, boxed, reached through `GWLP_USERDATA`. **One binary, both document
/// types** (decision 1): which pane a window is comes from [`grind_core::kind()`] reading the
/// file's bytes, and every message below belongs to one pane — [`with_sheet`], [`with_text`] — or
/// is answered for all here. The arms keep their own path, theme and dirty flag rather than a
/// shared header. A pane is boxed because the arms differ greatly in size.
enum Pane {
    Sheet(Box<Sheet>),
    Text(Box<Text>),
    /// **No document at all** — the welcome screen (`crate::welcome`) a window opened with no file
    /// shows. A third arm rather than an empty spreadsheet: a `Pane` says what the window *is*.
    /// Every handler asking `with_sheet`/`with_text` already gets `None` for the wrong pane, so
    /// the arm cost nothing there.
    Welcome(Box<Welcome>),
}

impl Pane {
    fn sheet_mut(&mut self) -> Option<&mut Sheet> {
        match self {
            Pane::Sheet(sheet) => Some(sheet),
            _ => None,
        }
    }

    fn text_mut(&mut self) -> Option<&mut Text> {
        match self {
            Pane::Text(text) => Some(text),
            _ => None,
        }
    }

    fn welcome_mut(&mut self) -> Option<&mut Welcome> {
        match self {
            Pane::Welcome(welcome) => Some(welcome),
            _ => None,
        }
    }

    /// The pending `WM_CHAR` high surrogate, whichever pane this is — `typed_char`'s slot,
    /// reached through `Pane` rather than `with_sheet`/`with_text` because a supplementary-plane
    /// character can arrive whether the window is showing a grid or a document.
    fn surrogate_mut(&mut self) -> &mut Option<u16> {
        match self {
            Pane::Sheet(sheet) => &mut sheet.surrogate,
            Pane::Text(text) => &mut text.surrogate,
            // The welcome screen takes no text at all, so nothing is ever left half-arrived on it.
            Pane::Welcome(welcome) => &mut welcome.surrogate,
        }
    }

    fn theme(&self) -> Theme {
        match self {
            Pane::Sheet(sheet) => sheet.theme,
            Pane::Text(text) => text.theme,
            Pane::Welcome(welcome) => welcome.theme,
        }
    }

    /// Re-read the theme and throw away everything that was made in the old palette.
    fn retheme(&mut self, theme: Theme) {
        match self {
            Pane::Sheet(sheet) => {
                sheet.theme = theme;
                sheet.field_brush = None;
            }
            Pane::Text(text) => text.theme = theme,
            Pane::Welcome(welcome) => welcome.theme = theme,
        }
    }

    fn path(&self) -> Option<PathBuf> {
        match self {
            Pane::Sheet(sheet) => sheet.path.clone(),
            Pane::Text(text) => text.path.clone(),
            Pane::Welcome(_) => None,
        }
    }

    /// What the document is called and where Save As starts: its path, or — for an imported
    /// workbook, which has none — the ODF name beside the workbook (`import.rs`).
    fn suggested(&self) -> Option<PathBuf> {
        match self {
            Pane::Sheet(sheet) => sheet.path.clone().or_else(|| sheet.imported.clone()),
            Pane::Text(text) => text.path.clone().or_else(|| text.imported.clone()),
            _ => self.path(),
        }
    }

    /// Which document kind this pane shows, for the Save dialog's filters and suggested name.
    /// `None` on the welcome screen — no document, so every caller decides what that means rather
    /// than being handed a guess; both are unreachable there (`menu::on_welcome`).
    fn kind(&self) -> Option<DocumentKind> {
        match self {
            Pane::Sheet(_) => Some(DocumentKind::Spreadsheet),
            Pane::Text(_) => Some(DocumentKind::Text),
            Pane::Welcome(_) => None,
        }
    }

    /// What the menu bar is being built for.
    fn surface(&self) -> menu::Surface {
        match self.kind() {
            Some(kind) => menu::Surface::Document(kind),
            None => menu::Surface::Welcome,
        }
    }

    fn set_dirty(&mut self, dirty: bool) {
        match self {
            Pane::Sheet(sheet) => sheet.dirty = dirty,
            Pane::Text(text) => text.dirty = dirty,
            // Nothing to be dirty: the welcome screen holds no document, so there is nothing for
            // the close question to ask about and nothing for a `*` to mark.
            Pane::Welcome(_) => {}
        }
    }

    fn dirty(&self) -> bool {
        match self {
            Pane::Sheet(sheet) => sheet.dirty,
            Pane::Text(text) => text.dirty,
            Pane::Welcome(_) => false,
        }
    }

    /// What the document is called, for a title bar and for the close question.
    fn document_name(&self) -> String {
        if matches!(self, Pane::Welcome(_)) {
            return welcome::TITLE.to_owned();
        }
        let path = self.suggested();
        match &path {
            Some(path) => path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.display().to_string()),
            None => "Untitled".to_owned(),
        }
    }

    fn title(&self) -> String {
        // The welcome screen is the application and not a document in it, so its caption is the
        // application's name once rather than "Grind — Grind".
        if matches!(self, Pane::Welcome(_)) {
            return APP_NAME.to_owned();
        }
        // A leading `*` for unsaved changes: Windows' own convention, and one that survives being
        // truncated in a taskbar button where a trailing marker would not.
        let mark = match self.dirty() {
            true => "*",
            false => "",
        };
        format!("{mark}{} — {APP_NAME}", self.document_name())
    }

    /// Write the document to `path`, whichever kind it is. The form is `Form::from_path`'s answer
    /// to the extension the user chose, inside the application's own `save_file`.
    fn save_file(&mut self, path: &Path) -> Result<(), String> {
        // Each application's own `save_file`, and each its own error type — reduced to a
        // sentence here, because what a caller does with a failed save is show it.
        let result = match self {
            Pane::Sheet(sheet) => sheet.app.save_file(path).map_err(|e| e.to_string()),
            Pane::Text(text) => text.app.save_file(path).map_err(|e| e.to_string()),
            Pane::Welcome(_) => Err("There is no document to save yet.".to_owned()),
        };
        match result {
            Ok(()) => {
                match self {
                    Pane::Sheet(sheet) => {
                        sheet.path = Some(path.to_owned());
                        sheet.imported = None;
                    }
                    Pane::Text(text) => {
                        text.path = Some(path.to_owned());
                        text.imported = None;
                    }
                    Pane::Welcome(_) => {}
                }
                self.set_dirty(false);
                Ok(())
            }
            Err(error) => Err(error),
        }
    }
}

/// Everything the **welcome screen** owns, which is almost nothing: no document, viewport,
/// selection, history or save. Where the cards are and which is pointed at — a press acts on
/// *release*, so it can be taken back by moving off the card.
struct Welcome {
    theme: Theme,
    page: welcome::Page,
    /// Which card the pointer is over, and which one is held down.
    hover: Option<usize>,
    pressed: Option<usize>,
    /// Which card the keyboard is on — always one, because there is nothing else here to focus
    /// and Enter has to mean something the moment the window opens.
    focus: usize,
    /// Unused here, and present because [`Pane::surrogate_mut`] is asked for one whichever pane
    /// the window is: a supplementary-plane `WM_CHAR` can arrive on any window, and this pane
    /// drops it rather than being a special case in `typed_char`.
    surrogate: Option<u16>,
}

impl Welcome {
    fn relayout(&mut self, width: f64, height: f64, dpi: u32) {
        self.page = welcome::Page { width, height, dpi };
    }
}

/// Everything the *spreadsheet* pane owns.
struct Sheet {
    app: grind_sheet::App,
    /// Where the last paint put the sheet tabs (`sheet::tabs`) — what a click on the status bar
    /// is tested against, so a tab is exactly as clickable as it looks. A `RefCell` because the
    /// paint borrows the state immutably.
    tabs: std::cell::RefCell<Vec<crate::sheet::tabs::Tab>>,
    path: Option<PathBuf>,
    /// For an imported workbook, which has no `path`: the ODF name beside it that the title shows
    /// and Save As starts from (`import.rs`). Cleared by the first save.
    imported: Option<PathBuf>,
    sheet: usize,
    geom: GridGeom,
    theme: Theme,
    /// An anchor and an active cell — presentation state; a range reaches `App` as two positions
    /// when something is done to it.
    selection: Selection,
    /// What a held mouse button is extending, if anything.
    drag: Option<Drag>,
    /// A header boundary being dragged to size a track: which, where the pointer was and how big
    /// the track was when it was taken. The live size is drawn from `geom`; the document is
    /// written once, on release, as one undo step.
    resize: Option<(Edge, f64, f64)>,
    /// The child `EDIT` that *is* the name box while somebody types in it — created hidden and
    /// shown over the drawn box on demand; `sheet/draw.rs` paints the box otherwise, so
    /// `--render-to` shows an address with no control.
    name_box: HWND,
    name_box_open: bool,
    /// The child `EDIT` holding an edit in progress — the in-cell editor and the formula bar,
    /// **one control in two places**. A widget holding a keystroke, not the document (decision 2):
    /// emptied into [`grind_sheet::App::enter`] and hidden.
    editor: HWND,
    /// Whether that control is on the formula bar rather than on the cell — true when the edit
    /// began by clicking the bar, and when the active cell is scrolled out of sight.
    editor_on_bar: bool,
    /// Ready, Enter or Edit. Decided by `sheet/state.rs`, which is a pure function; this is the
    /// one thing it needs remembered between two keystrokes — and it is also the answer to
    /// "is the editor up", which is why there is no second flag saying so.
    mode: state::Mode,
    /// Unsaved changes. The core deliberately has none — it has *undo*, which answers a
    /// different question — so the flag lives here, and it is set by [`Changed`] rather than by
    /// each handler remembering to.
    dirty: bool,
    /// What the notice bar says, or `None` for a document with nothing to say about itself.
    /// Always set through [`Sheet::say`], which is what keeps it and `geom.banner_h` agreeing.
    banner: Option<String>,
    /// The word Edit ▸ Find… last asked for — what F3 and Shift+F3 step through and Replace…
    /// offers. Empty until a search.
    needle: String,
    /// What is being offered to somebody typing a formula, and which offer Tab would take —
    /// `sheet/assist.rs`, recomputed from the editor's text on every keystroke rather than
    /// updated in place.
    assist: assist::Assist,
    /// The assist band's runs, as [`assist::band`] renders `assist`. Kept here rather than
    /// computed in the paint because their emptiness decides `geom.hint_h` — a band with height
    /// and nothing in it is a gap, one with runs and no height draws over the headers. Always set
    /// through [`Sheet::hint`], as [`Sheet::say`] sets the notice bar.
    hint: Vec<assist::Piece>,
    /// The reference being pointed at while a formula is typed (point mode): where its text is in
    /// the editor, as bytes, and the cell it names.
    pointing: Option<(std::ops::Range<usize>, Pos)>,
    /// Whether a formula cell draws its formula instead of its result (View ▸ Show Formulas).
    formulas: bool,
    /// Whether the formula bar shows the friendly *reading* of a formula — View ▸ Friendly
    /// Formulas. Presentation state; the stored formula stays ODF's (R1).
    friendly: bool,
    /// The face the two child `EDIT`s are set in. Owned here because a `WM_SETFONT` does not
    /// take a copy: the handle has to outlive every paint of the control, and be deleted after
    /// it.
    ui_font: Option<Font>,
    /// The ground the child `EDIT`s are painted on, answered to `WM_CTLCOLOREDIT`. Built on
    /// demand and thrown away when the theme changes, because a brush is a colour and the
    /// colour is the theme's.
    field_brush: Option<Brush>,
    /// A `WM_CHAR` high surrogate waiting for its low half — `typed_char`'s state, per window
    /// rather than static. Only in Ready mode: once an edit is open, the native `EDIT` assembles
    /// the pair itself.
    surrogate: Option<u16>,
    /// `doc/view-modes.md`'s two overlays — presentation state like `selection`, never told to the
    /// core and asked for fresh on every `get_viewport_with`, which keeps a save with every
    /// overlay on byte-identical.
    overlays: grind_sheet::view::Overlays,
    /// Which format-strip control the pointer is over, and which one is held down — the text
    /// pane's `hover`/`pressed` pair, for the grid's strip: a press acts on *release*, so it can
    /// be taken back by moving off the control, and a control responds to the pointer at all.
    format_hover: Option<format::Control>,
    format_pressed: Option<format::Control>,
    /// The selected chart (`doc/chart-handling.md`), as its sheet and its index there: outlined
    /// and handled, and what the arrows, Delete and the Data menu's chart verbs mean. Read through
    /// [`Sheet::chart_now`], which lets go of one on another sheet or one deleted.
    chart_selected: Option<(usize, usize)>,
    /// A chart held by the pointer; the document is written once, on release.
    chart_grab: Option<crate::sheet::chart::Grab>,
    /// Spelling: the person's remembered choice (`grind_spell::preference`), so Check Spelling off
    /// here is off in every spreadsheet; what it came to for this document; and the dictionary
    /// Ignore All adds to. `checked` is `None` when nothing is checked, and nothing is underlined.
    spelling: grind_spell::Setting,
    /// The last setting that was not Off — what Shift+F7 turns spelling back on to.
    spelling_on: grind_spell::Setting,
    checked: Option<grind_spell::Choice>,
    speller: Option<std::sync::Arc<grind_spell::Speller>>,
    /// The guess has read all a guess reads and cannot change, so edits stop sampling it.
    settled: bool,
    /// The misspelt word F7 last stopped on, so the next F7 goes on to the word after it — in
    /// the same cell when it has another — rather than back to the cell's first.
    spell_stop: Option<grind_sheet::Misspelling>,
}

/// What a click on the strip landed on. The two fields there are *drawn* until somebody clicks
/// one, at which point the control that was hiding behind the drawing appears over it — which
/// is what lets `--render-to` show both with no window anywhere.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Strip {
    NameBox,
    FormulaBar,
    Neither,
}

/// What `button_down` found under the pointer, before it decides what a click means — the
/// strip's own two fields, or an autofilter's dropdown button caught the same way, before the
/// click would otherwise have become an ordinary cell selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Click {
    Strip(Strip),
    /// The button in this column of the filter's heading row.
    FilterButton(u32),
}

/// Which child a relayed key belongs to, and — for the editor — the mode to read it in.
#[derive(Clone, Copy, Debug)]
enum Focused {
    NameBox,
    Editor(state::Mode),
}

/// What a drag started on, which decides what moving the mouse extends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Drag {
    Cells,
    Cols,
    Rows,
}

impl Sheet {
    /// Attach what [`Sheet::spelling`] says to the document — on open and whenever the choice
    /// changes. A document in a language with no dictionary says so on the notice bar rather
    /// than having every word underlined.
    fn check_spelling(&mut self) {
        self.settled = false;
        match self.spelling.apply(&self.app) {
            Ok(Some((choice, speller))) => {
                self.checked = Some(choice);
                self.speller = Some(speller);
            }
            Ok(None) => {
                self.checked = None;
                self.speller = None;
            }
            Err(why) => {
                self.checked = None;
                self.speller = None;
                self.say(Some(notice::spelling_unchecked(&why.to_string())));
            }
        }
    }

    /// Choose a spelling setting, remember it for every spreadsheet, and apply it.
    fn set_spelling(&mut self, setting: grind_spell::Setting) {
        self.spelling = setting;
        if setting != grind_spell::Setting::Off {
            self.spelling_on = setting;
        }
        self.say(None);
        if let Err(why) =
            grind_spell::preference::save(grind_core::DocumentKind::Spreadsheet, setting)
        {
            self.say(Some(notice::spelling_not_remembered(&why.to_string())));
        }
        self.check_spelling();
    }

    /// While the language is only a guess, guess again as the document grows
    /// (`grind_spell::reguess`), until it has more words than a guess reads.
    fn reconsider_spelling(&mut self) {
        let Some(choice) = self.checked else {
            return;
        };
        if self.settled
            || self.spelling != grind_spell::Setting::Automatic
            || !matches!(
                choice.source,
                grind_spell::Source::Guessed | grind_spell::Source::Default
            )
        {
            return;
        }
        let words = grind_core::spell::Spelled::sample(&self.app).len();
        if grind_spell::reguess(&self.app, self.spelling, choice, words) {
            self.check_spelling();
        } else if words >= grind_core::spell::GUESS_SAMPLE {
            self.settled = true;
        }
    }

    /// Rebuild the geometry for the current client size and DPI — from the *document's* lengths,
    /// never scaled from the last answer, so a window dragged between monitors and back is
    /// pixel-identical to one that never moved.
    fn relayout(&mut self, width: f64, height: f64, dpi: u32) {
        // The one place the sheet index is checked, since every path changing the document ends in
        // a relayout: deleting a sheet, undoing an insertion or opening a smaller document can
        // leave it past the end.
        self.sheet = self.sheet.min(self.app.sheet_count().saturating_sub(1));
        let widths = self.app.col_widths(self.sheet).unwrap_or_default();
        let heights = self.app.row_heights(self.sheet).unwrap_or_default();
        // Hiding is `table:visibility`, not a width of zero, so it is a second question and
        // three answers: a column hidden by hand, a row hidden by hand, and a row a *filter*
        // excludes — `App` keeps the last two apart and this shell draws both as gone.
        let hidden_cols = self.app.hidden_cols(self.sheet).unwrap_or_default();
        let mut hidden_rows = self
            .app
            .manually_hidden_rows(self.sheet)
            .unwrap_or_default();
        hidden_rows.extend(self.app.hidden_rows(self.sheet).unwrap_or_default());
        self.geom = GridGeom {
            // The format strip is one Fluent strip tall, like the name box's strip under it and
            // the text pane's own — three bands of chrome that measure the same in one window.
            format_h: scale(draw::STRIP_H, dpi),
            strip_h: scale(draw::STRIP_H, dpi),
            banner_h: banner_h(self.banner.as_deref(), dpi),
            hint_h: hint_h(&self.hint, dpi),
            header_w: scale(draw::HEADER_W, dpi),
            header_h: scale(draw::HEADER_H, dpi),
            status_h: scale(draw::STATUS_H, dpi),
            // Every cell track is built from the document's lengths and then scaled by the zoom —
            // the arithmetic downstream stays in one space (`Sizes::scaled`).
            cols: Sizes::from_lengths(
                scale(draw::COL_W, dpi),
                MAX_COLS,
                &widths,
                &hidden_cols,
                mm_to_px(dpi),
            )
            .scaled(self.geom.zoom),
            rows: Sizes::from_lengths(
                scale(draw::ROW_H, dpi),
                MAX_ROWS,
                &heights,
                &hidden_rows,
                mm_to_px(dpi),
            )
            .scaled(self.geom.zoom),
            first_row: self.geom.first_row,
            first_col: self.geom.first_col,
            width,
            height,
            dpi,
            zoom: self.geom.zoom,
        };
        // Every row with no height of its own grows to hold what is in it — a wrapping cell, or
        // one in a larger face — measured in GDI, the engine that draws it.
        if let Some(dib) = gdi::Dib::new(1, 1) {
            let metrics = measure::CellMetrics {
                dc: dib.dc(),
                face: face(),
                dpi,
                default_pt: theme::text::CELL * 72.0 / 96.0,
                zoom: self.geom.zoom,
            };
            let zoom = self.geom.zoom;
            let row_h = scale(draw::ROW_H, dpi) * zoom;
            // A cell's padding either side (`draw`'s `PAD`, 4 pixels at 100%).
            let pad = scale(4.0, dpi).round();
            let grown = grind_sheet::autoheight::grown_rows(
                &self.app,
                self.sheet,
                &|col| self.geom.cols.size_of(col),
                pad,
                2.0 * scale(2.0, dpi),
                row_h,
                &metrics,
            );
            if !grown.is_empty() {
                let mut sizes: Vec<(u32, f64)> = grown;
                // The document's own heights, and its hidden rows, go on after — `Sizes::new`
                // keeps the last entry given for a row, so they win.
                let to_px = mm_to_px(dpi);
                sizes.extend(heights.iter().filter_map(|(row, length)| {
                    Some((
                        *row,
                        grind_core::style::length_mm(length).map(&to_px)? * zoom,
                    ))
                }));
                sizes.extend(hidden_rows.iter().map(|row| (*row, 0.0)));
                self.geom.rows = Sizes::new(row_h, MAX_ROWS, sizes);
            }
        }
        // A window that grew may now show past the last row; clamp rather than leave the view
        // parked in blank space below the sheet.
        self.geom.first_row = self.geom.first_row.min(self.geom.max_first_row());
        self.geom.first_col = self.geom.first_col.min(self.geom.max_first_col());
    }

    /// What the sheet's occupied region is, as far as navigation cares.
    ///
    /// A page keeps one row of context, which is what every other grid does.
    fn extent(&self) -> keymap::Extent {
        let (rows, cols) = self.app.used_extent(self.sheet).unwrap_or((0, 0));
        let visible = u32::try_from(self.geom.page_rows()).unwrap_or(1);
        keymap::Extent {
            rows,
            cols,
            page: visible.saturating_sub(1).max(1),
        }
    }

    /// Do what a keystroke asked for. [`keymap::Action::GoTo`] is not here: it opens a window,
    /// and nothing that touches a window happens while the state is borrowed (decision 7).
    fn act(&mut self, action: keymap::Action) {
        match action {
            keymap::Action::Move { motion, extend } => {
                let extent = self.extent();
                let occupied = grind_sheet::nav::occupied(&self.app, self.sheet);
                let to = keymap::moved(self.selection, motion, extend, extent, &occupied);
                // A hidden track is drawn as gone, so a cursor may not stop on one — see
                // `keymap::onto_visible`, which is the rule and has the tests.
                let to = keymap::onto_visible(to, motion, &self.geom.rows, &self.geom.cols);
                // A merge is one cell: a step leaves it from its far edge.
                let merges = grind_sheet::nav::merges(&self.app, self.sheet);
                self.selection =
                    grind_sheet::nav::through_merges(self.selection, to, motion, &merges);
            }
            // Everything the sheet *uses*, with the active cell at A1 so that the view goes
            // home rather than to the far corner. An empty sheet selects the one cell it has.
            keymap::Action::SelectAll => {
                self.selection = grind_sheet::nav::all(&self.app, self.sheet)
            }
            keymap::Action::GoTo => {}
        }
        self.reveal();
    }

    /// Scroll the least it takes to put the active cell in the body: one clamp per axis — the view
    /// may not start after the active cell, nor before the least first track that shows it. A cell
    /// already in view falls between and nothing moves. Also where a single cell landing inside a
    /// merge is moved onto its top-left cell, since every selection move ends here.
    fn reveal(&mut self) {
        let merges = grind_sheet::nav::merges(&self.app, self.sheet);
        self.selection = grind_sheet::nav::onto_merge(self.selection, &merges);
        let (body, active) = (self.geom.body(), self.selection.active);
        let g = &mut self.geom;
        let need = g.rows.start_showing(active.row, body.h);
        g.first_row = g.first_row.clamp(need, active.row.max(need));
        let need = g.cols.start_showing(active.col, body.w);
        g.first_col = g.first_col.clamp(need, active.col.max(need));
        g.first_row = g.first_row.min(g.max_first_row());
        g.first_col = g.first_col.min(g.max_first_col());
    }

    /// Extend a drag to whatever is under the pointer, keeping the anchor where it was.
    fn drag_to(&mut self, hit: Hit) {
        let Some(drag) = self.drag else { return };
        let active = match (drag, hit) {
            (Drag::Cells, Hit::Cell { row, col }) => Pos::new(row, col),
            // A drag that leaves the grid keeps to its own axis: dragging along a header band
            // and dipping into the cells still selects whole columns.
            (Drag::Cols, Hit::Cell { col, .. } | Hit::ColHeader(col)) => Pos::new(0, col),
            (Drag::Rows, Hit::Cell { row, .. } | Hit::RowHeader(row)) => Pos::new(row, 0),
            _ => return,
        };
        if self.selection.active != active {
            self.selection.active = active;
            self.reveal();
        }
    }

    /// Move the active cell one step, after an edit that committed with an arrow key.
    fn move_by(&mut self, dir: Dir) {
        self.act(keymap::Action::Move {
            motion: keymap::Motion::By(dir),
            extend: false,
        });
    }

    /// The selected chart's index, if it is on the sheet showing and still exists.
    fn chart_now(&self) -> Option<usize> {
        let (sheet, index) = self.chart_selected?;
        let count = self.app.charts(self.sheet).map_or(0, |charts| charts.len());
        (sheet == self.sheet && index < count).then_some(index)
    }

    /// The chart a chart verb means: the selected one, else the sheet's last.
    fn chart_target(&self) -> Option<usize> {
        self.chart_now().or_else(|| {
            self.app
                .charts(self.sheet)
                .ok()
                .and_then(|charts| charts.len().checked_sub(1))
        })
    }

    /// Put something in the notice bar, or take it away — the sentence and the height change
    /// together, since one without the other is nothing drawn or a bar nobody can dismiss.
    fn say(&mut self, notice: Option<String>) {
        self.geom.banner_h = banner_h(notice.as_deref(), self.geom.dpi);
        self.banner = notice;
    }

    /// Recompute the assist band from what the editor holds, setting its height with it, as
    /// [`Sheet::say`] does for the notice — the runs and the height change together. `caret` is a
    /// **byte** offset into `text` ([`state::byte_at`] converts `EM_GETSEL`'s). Returns whether
    /// the height changed: repaint the strip, or relayout, since the grid moves down when the band
    /// appears.
    fn assist(&mut self, text: &str, caret: usize) -> bool {
        let names: Vec<String> = self.app.names().into_iter().map(|(name, _)| name).collect();
        self.assist.refresh(text, caret, &names);
        let pieces = assist::band(&self.assist, self.friendly);
        self.hint(pieces)
    }

    /// Put runs in the assist band, or take it away, and move the height with them.
    fn hint(&mut self, pieces: Vec<assist::Piece>) -> bool {
        let was = self.geom.hint_h;
        self.geom.hint_h = hint_h(&pieces, self.geom.dpi);
        self.hint = pieces;
        was != self.geom.hint_h
    }

    /// An edit ended, by any door: no offers, no signature, no band.
    fn assist_done(&mut self) {
        self.assist.clear();
        self.hint(Vec::new());
    }

    /// Where the edit control goes: over the active cell, unless it is scrolled out of sight or
    /// the edit began on the formula bar, where the text then is.
    fn editor_rect(&self) -> Rect {
        let active = self.selection.active;
        let cell = self.geom.editor_rect(active.row, active.col);
        match self.editor_on_bar || cell.w <= 0.0 || cell.h <= 0.0 {
            true => self.geom.formula_rect(),
            false => cell,
        }
    }
}

/// Everything the *word processor's* pane owns — smaller than [`Sheet`], but holding a
/// [`text::geom::Flow`] that had to be *measured*, which is why the font cache is here: state,
/// expensive, and the measuring pane's.
struct Text {
    app: grind_text::App,
    path: Option<PathBuf>,
    /// For an imported markdown file, which has no `path`: the ODF name beside it, where Save As
    /// starts — `Sheet::imported`'s twin.
    imported: Option<PathBuf>,
    theme: Theme,
    dirty: bool,
    /// The window's bands and how far down the document the body starts.
    page: Page,
    /// Where every block sits, measured — rebuilt when the document, the width or the DPI changes,
    /// not per paint.
    flow: Flow,
    /// Where every block in a table cell sits across the column and the width it is set at
    /// (`text::geom::across`), rebuilt with `flow` and read by [`Text::faces`] — which is how a
    /// caret motion inside a cell measures the cell rather than the whole column.
    across: HashMap<usize, text::geom::Across>,
    /// The caret and the other end of the selection. Presentation state: the core is never told
    /// about it, exactly as the grid's `Selection` is never told to `grind_sheet::App`.
    caret: Caret,
    anchor: Caret,
    /// The column Down and Up aim for, in pixels, kept across a run of vertical keystrokes and
    /// dropped when the caret moves horizontally — a property of the keystrokes, not the document,
    /// which is why `App::caret_line` takes it.
    goal_x: Option<f32>,
    /// The fonts everything is measured and drawn with (`metrics.rs`). `None` only before the
    /// first `WM_CREATE`, and rebuilt whole on `WM_DPICHANGED` — a font is a size in *pixels*,
    /// so every one of them is wrong on a monitor with different scaling.
    fonts: Option<Fonts>,
    /// The DPI `fonts` was built for, which is the only reason to build another.
    fonts_dpi: u32,
    /// Whether a drag is extending the selection.
    dragging: bool,
    /// Whether a caret is drawn by the paint — only for `render`'s windowless frame, which has no
    /// system caret. In a real window the system caret (`place_system_caret`) is the caret and
    /// this stays `true`, ignored.
    caret_on: bool,
    banner: Option<String>,
    /// What [`grind_text::App::type_markdown`] said the next character must be set in — the style
    /// a completed `**bold**` leaves behind, carried across keystrokes.
    resume: Option<grind_text::CharStyle>,
    /// A `WM_CHAR` high surrogate waiting for its low half — `Sheet::surrogate`'s twin, and the
    /// one that matters every keystroke rather than only at the start of an edit: this pane has
    /// no native control to assemble a pair for it.
    surrogate: Option<u16>,
    /// `doc/view-modes.md`'s name overlay, this pane's own — `:names`' equivalent, a bookmark
    /// drawn beside the text it anchors. Presentation state, exactly like `Sheet::overlays`.
    show_names: bool,
    /// The word [`text_find`] last asked for, which F3 and Shift+F3 step through and Replace
    /// offers back.
    needle: String,
    /// Which format-strip control is held down, and therefore the one a release over it will
    /// activate. See `text_button_down`.
    pressed: Option<text::geom::StripHit>,
    /// Which format-strip control the pointer is over. A row of drawn buttons with no hover
    /// feedback is what most gives a custom-painted window away; repainting only when this changes
    /// keeps a mouse move to one comparison.
    hover: Option<text::geom::StripHit>,
    /// Spelling (`doc/spelling.md`): the session's choice in View ▸ Spelling Language…, what it
    /// came to for this document, and the dictionary Ignore All adds to. `checked` is `None`
    /// when nothing is checked, which is also when nothing is underlined.
    spelling: grind_spell::Setting,
    checked: Option<grind_spell::Choice>,
    speller: Option<std::sync::Arc<grind_spell::Speller>>,
}

impl Text {
    /// Choose a spelling setting, remember it for every text document, and apply it.
    fn set_spelling(&mut self, setting: grind_spell::Setting) {
        self.spelling = setting;
        self.say(None);
        if let Err(why) = grind_spell::preference::save(DocumentKind::Text, setting) {
            self.say(Some(notice::spelling_not_remembered(&why.to_string())));
        }
        self.check_spelling();
    }

    /// Attach what [`Text::spelling`] says to the document — on open and whenever the choice
    /// changes. A document in a language with no dictionary says so on the notice bar rather
    /// than having every word underlined.
    fn check_spelling(&mut self) {
        match self.spelling.apply(&self.app) {
            Ok(Some((choice, speller))) => {
                self.checked = Some(choice);
                self.speller = Some(speller);
            }
            Ok(None) => {
                self.checked = None;
                self.speller = None;
            }
            Err(why) => {
                self.checked = None;
                self.speller = None;
                self.say(Some(notice::spelling_unchecked(&why.to_string())));
            }
        }
    }

    /// While the language is only a guess, guess again as the document grows
    /// (`grind_spell::reguess`) — the first sentences typed into a new document deciding it.
    fn reconsider_spelling(&mut self) {
        if let Some(choice) = self.checked
            && matches!(
                choice.source,
                grind_spell::Source::Guessed | grind_spell::Source::Default
            )
            && grind_spell::reguess(&self.app, self.spelling, choice, self.app.counts().words)
        {
            self.check_spelling();
        }
    }

    /// The misspelt word at `at` — `at` anywhere from its first character to just past its last.
    fn misspelt_at(&self, at: Caret) -> Option<grind_text::Misspelling> {
        self.app
            .misspellings(at.block..at.block + 1)
            .into_iter()
            .find(|m| m.offset <= at.offset && at.offset <= m.offset + m.len)
    }

    /// Rebuild the bands, the fonts if the monitor changed, and the flow — from the document,
    /// never scaled from the last answer, as [`Sheet::relayout`] does.
    fn relayout(&mut self, width: f64, height: f64, dpi: u32) {
        self.page = Page {
            width,
            height,
            banner_h: match self.banner {
                Some(_) => scale(text::geom::BANNER_H, dpi),
                None => 0.0,
            },
            strip_h: scale(text::geom::STRIP_H, dpi),
            status_h: scale(text::geom::STATUS_H, dpi),
            dpi,
            scroll: self.page.scroll,
        };
        if self.fonts.is_none() || self.fonts_dpi != dpi {
            // The old cache goes first: it owns every `HFONT` in it, and a shell that kept both
            // would leak one per monitor change — `doc/windows-shell.md` names exactly that as
            // this shell's classic `unsafe` risk.
            self.fonts = Fonts::new(scale(f64::from(TEXT_PX), dpi));
            self.fonts_dpi = dpi;
        }
        self.reflow();
        self.page.scroll = self
            .page
            .scroll
            .clamp(0.0, self.flow.limit(self.page.body().h));
    }

    /// Measure every block and stack them.
    ///
    /// ponytail: the **whole document** is laid out, so a five-hundred-page one is measured on
    /// every resize. That is what makes the scrollbar honest, and it is fast enough for everything
    /// in `text/tests/data/`. The upgrade is an estimated height for unseen blocks, replaced as
    /// they scroll past, when a resize is visibly slow.
    fn reflow(&mut self) {
        let dpi = self.page.dpi;
        let (_, column) = self.page.text_column();
        // Before the faces, which read it: a cell's measure is a fact about the table's shape
        // and the column, and nothing about how tall anything is.
        self.across = text::geom::across(&self.app, column, dpi);
        let Some(faces) = self.faces() else {
            return;
        };
        // A picture is measured from its decoded pixels — fit the column, keep the aspect ratio,
        // never larger than its own size (`flow_of`). `None` when the block is no picture or WIC
        // cannot read it (R5), so a bad picture takes its placeholder's room rather than stopping
        // the document.
        let picture = |view: &grind_text::BlockView, width: f64| -> Option<f64> {
            let (image, caption) = grind_text::picture_of(view)?;
            let (w, h) = image::size(&image.data)?;
            let picture_h = f64::from(h) * (width.min(f64::from(w)) / f64::from(w));
            match caption {
                Some(text) if !text.is_empty() => {
                    let body = faces.face(&grind_text::BlockKind::Paragraph, None);
                    let gap = scale(text::geom::CAPTION_GAP, dpi);
                    Some(
                        picture_h
                            + gap
                            + body.wrapped_height(self.fonts.as_ref()?.dc(), text, width),
                    )
                }
                _ => Some(picture_h),
            }
        };
        self.flow = text::geom::flow_of(&self.app, &faces, column, dpi, &picture);
    }

    /// Whether anything is selected at all.
    fn has_selection(&self) -> bool {
        self.anchor != self.caret
    }

    /// The selection's two ends in document order — what every verb over a range is given.
    fn range(&self) -> (Caret, Caret) {
        text::keymap::ordered(self.anchor, self.caret)
    }

    /// How many characters a block holds. Asked of the viewport rather than remembered, because
    /// a block's length changes under every edit.
    fn block_len(&self, index: usize) -> usize {
        grind_text::caret::block_len(&self.app, index)
    }

    fn last_caret(&self) -> Caret {
        grind_text::caret::end(&self.app)
    }

    /// The faces this document is set in, at the current measure — built per call, since it
    /// borrows the font cache; the fonts are cached, so this is a few map lookups.
    fn faces(&self) -> Option<Faces<'_>> {
        let fonts = self.fonts.as_ref()?;
        let (_, width) = self.page.text_column();
        Some(Faces::new(
            fonts,
            width,
            text::geom::spacing(self.page.dpi),
            &self.across,
        ))
    }

    /// One block's lines, measured in its own face — computed and thrown away, since a kept layout
    /// goes stale on the next keystroke.
    fn layout_of(&self, index: usize) -> Option<Layout> {
        let faces = self.faces()?;
        let viewport = self.app.get_viewport(index..index + 1);
        let view = viewport.get(index)?;
        let (width, metrics) =
            grind_text::Faces::of(&faces, index, &view.kind, view.style.as_deref());
        self.app.layout_block(index, width, metrics).ok()
    }

    /// Scroll the least it takes to put the caret's own line on screen, and no more.
    fn reveal(&mut self) {
        let Some(slot) = self.flow.slot(self.caret.block).copied() else {
            return;
        };
        let target = match self.layout_of(self.caret.block) {
            Some(layout) => {
                let line = layout.lines()[layout.line_at(self.caret.offset)];
                (slot.top + f64::from(line.top), f64::from(line.height))
            }
            None => (slot.top, slot.height),
        };
        let page = self.page.body().h;
        self.page.scroll = self.flow.follow(self.page.scroll, page, target);
    }

    /// The caret's x within its line, in pixels — what Down and Up aim for.
    fn caret_x(&self) -> Option<f32> {
        let faces = self.faces()?;
        self.app.caret_x(self.caret, &faces).ok()
    }

    /// Where the caret is drawn, in client pixels, and its size — `text/draw.rs`'s geometry
    /// without a `Frame`. `position_ime_composition` wants the point, so an IME's candidate list
    /// appears at the caret; `place_system_caret` wants all four, since `CreateCaret` takes a size
    /// and a heading's caret is taller.
    fn caret_geometry(&self) -> Option<(i32, i32, i32, i32)> {
        let slot = self.flow.slot(self.caret.block).copied()?;
        let layout = self.layout_of(self.caret.block)?;
        let line = layout
            .lines()
            .get(layout.line_at(self.caret.offset))
            .copied()?;
        let (column_x, _) = self.page.text_column();
        let body = self.page.body();
        let x = column_x + slot.indent + f64::from(layout.x_at(self.caret.offset));
        let y = body.y + slot.top + f64::from(line.top) - self.page.scroll;
        let width = scale(text::draw::CARET_W, self.page.dpi).max(1.0);
        let height = f64::from(line.height).max(1.0);
        Some((
            x.round() as i32,
            y.round() as i32,
            width.round() as i32,
            height.round() as i32,
        ))
    }

    /// One block's plain text, for the questions that are about characters rather than about
    /// lines — word motion, and how long a block is.
    fn block_text(&self, index: usize) -> String {
        self.app
            .get_viewport(index..index + 1)
            .get(index)
            .map(|view| view.text.clone())
            .unwrap_or_default()
    }

    /// Where a motion lands, without moving anything. Every vertical answer is
    /// [`grind_text::App`]'s — `caret_line` and `caret_line_bounds` through this pane's [`Faces`]
    /// (`doc/text-layout.md`); the horizontal half, about characters, is here.
    fn moved(&self, motion: text::keymap::Motion, goal: f32) -> Caret {
        use text::keymap::Motion;
        let at = self.caret;
        match motion {
            // Off the end of a block is the start of the next one: a document is one flow, not a
            // list of boxes, which is the same rule `App::caret_line` follows.
            Motion::Char(step) => grind_text::caret::step(&self.app, at, i32::from(step)),
            Motion::Word(step) => {
                let forward = step > 0;
                let text = self.block_text(at.block);
                match text::keymap::word_boundary(&text, at.offset, forward) {
                    Some(offset) => Caret {
                        block: at.block,
                        offset,
                    },
                    // At the block's own end in that direction, carry one character into the
                    // neighbour — which is where the next word begins.
                    None => self.moved(Motion::Char(step), goal),
                }
            }
            Motion::Line(delta) => match self.faces() {
                Some(faces) => self.app.caret_line(at, delta, goal, &faces).unwrap_or(at),
                None => at,
            },
            Motion::LineStart | Motion::LineEnd => match self.faces() {
                Some(faces) => match self.app.caret_line_bounds(at, &faces) {
                    Ok((start, end)) => match motion {
                        Motion::LineStart => start,
                        _ => end,
                    },
                    Err(_) => at,
                },
                None => at,
            },
            Motion::DocStart => START,
            Motion::DocEnd => self.last_caret(),
        }
    }

    /// Move the caret, extending or collapsing the selection. The **goal column** survives a run
    /// of vertical keystrokes, so walking Down through a short line comes back to the column it
    /// started in.
    fn navigate(&mut self, motion: text::keymap::Motion, extend: bool) {
        let vertical = matches!(motion, text::keymap::Motion::Line(_));
        let goal = match vertical {
            true => self.goal_x.or_else(|| self.caret_x()),
            false => None,
        };
        self.resume = None;
        self.caret = self.moved(motion, goal.unwrap_or(0.0));
        if !extend {
            self.anchor = self.caret;
        }
        self.goal_x = goal;
        self.reveal();
    }

    /// Put the caret somewhere, forgetting the goal column, the selection (unless `extend`) and
    /// any style pending for the next character — a Bold pressed with nothing selected belongs to
    /// where it was pressed. Typing sets the style again after placing the caret (`text_type`).
    fn place(&mut self, at: Caret, extend: bool) {
        self.resume = None;
        self.caret = at;
        if !extend {
            self.anchor = at;
        }
        self.goal_x = None;
        self.reveal();
    }

    /// Which caret position a point is nearest — **nearest, never nothing** ([`Flow::at`]): a
    /// click in a line's margin lands at its near end, since a caret refusing to move is a bug
    /// nobody can see the cause of.
    fn caret_at(&self, x: f64, y: f64) -> Option<Caret> {
        let body = self.page.body();
        let (column_x, _) = self.page.text_column();
        grind_text::caret::hit(
            &self.flow,
            x - column_x,
            y - body.y + self.page.scroll,
            |block| self.layout_of(block),
        )
    }

    /// How many lines Page Up or Down moves: body lines, one short of a screenful, keeping a line
    /// of context across the jump, as the grid's page does.
    fn page_lines(&self) -> isize {
        let height = self
            .fonts
            .as_ref()
            .map(|fonts| fonts.body_px() * 1.3)
            .unwrap_or(20.0)
            .max(1.0);
        ((self.page.body().h / height) as isize - 1).max(1)
    }

    /// Put something in the notice bar, or take it away. The sentence and the *height* change
    /// together, which is the same rule [`Sheet::say`] follows.
    fn say(&mut self, notice: Option<String>) {
        self.page.banner_h = match notice {
            Some(_) => scale(text::geom::BANNER_H, self.page.dpi),
            None => 0.0,
        };
        self.banner = notice;
    }
}

/// How tall the notice bar is for a given notice — zero for no notice, which is the whole of
/// how the bar appears and disappears.
fn banner_h(notice: Option<&str>, dpi: u32) -> f64 {
    match notice {
        Some(_) => scale(draw::BANNER_H, dpi),
        None => 0.0,
    }
}

/// The same question for the assist band: zero when there is nothing to assist with.
fn hint_h(hint: &[assist::Piece], dpi: u32) -> f64 {
    match hint.is_empty() {
        true => 0.0,
        false => scale(draw::HINT_H, dpi),
    }
}

/// The bridge from the core to the window: *something changed* (rule 3). It **posts** rather than
/// sends: `App::mutate` notifies with its write lock dropped but still inside the changing call,
/// and a `SendMessageW` there would re-enter this procedure while a handler holds `&mut State`.
/// The window is an `isize`, not an `HWND`, so this is `Send + Sync` without an unsafe promise, as
/// [`grind_core::Observer`] requires.
struct Changed(isize);

impl grind_core::Observer for Changed {
    fn changed(&self) {
        // SAFETY: posting to a window that has already been destroyed is defined — it fails and
        // returns an error, which is why the result is discarded rather than checked.
        unsafe {
            let _ = PostMessageW(
                Some(HWND(self.0 as *mut std::ffi::c_void)),
                WM_DOC_CHANGED,
                WPARAM(0),
                LPARAM(0),
            );
        }
    }
}

/// A document, read, with a window's worth of state round it and no window — shared by [`run`] and
/// [`render`], so a render is a second caller of every answer the window uses (decision 5). **The
/// caller decides the pane**, from `grind_core::kind` reading the bytes. `None` is the welcome
/// screen.
fn opened(kind: Option<DocumentKind>, path: Option<PathBuf>, theme: Theme) -> Result<Pane, String> {
    let Some(kind) = kind else {
        return Ok(Pane::Welcome(Box::new(opened_welcome(theme))));
    };
    match kind {
        DocumentKind::Spreadsheet => Ok(Pane::Sheet(Box::new(opened_sheet(path, theme)?))),
        DocumentKind::Text => Ok(Pane::Text(Box::new(opened_text(path, theme)?))),
        // Recognised and refused, which is what `DocumentKind::Presentation` is *for*: this
        // suite has no presentation application, and saying so is better than parsing one as an
        // empty document of some other kind.
        DocumentKind::Presentation => {
            Err("This is a presentation, and the suite has no application for one.".to_owned())
        }
    }
}

/// The welcome screen. Infallible, unlike the other two: there is nothing to read.
fn opened_welcome(theme: Theme) -> Welcome {
    Welcome {
        theme,
        page: welcome::Page::default(),
        hover: None,
        pressed: None,
        // The first card, so Enter on a fresh window makes a spreadsheet — the default answer to a
        // question that was at least put.
        focus: 0,
        surrogate: None,
    }
}

/// Read the file at `path` with `open`: imported to flat ODF under its ODF name, or as it is.
/// Returns the import, which leaves the document unsaved with no path for Save to write over.
fn read<E: std::fmt::Display>(
    path: &Path,
    open: impl FnOnce(&str, &[u8]) -> Result<(), E>,
) -> Result<Option<grind_open::Imported>, String> {
    let fail = |error: &dyn std::fmt::Display| format!("{}: {error}", path.display());
    let bytes = std::fs::read(path).map_err(|error| fail(&error))?;
    let imported = grind_open::import_file(path, &bytes).transpose()?;
    match &imported {
        Some(imported) => open(&imported.name, &imported.odf),
        None => open(&path.display().to_string(), &bytes),
    }
    .map_err(|error| fail(&error))?;
    Ok(imported)
}

/// The word processor's pane, on a document or on nothing.
fn opened_text(path: Option<PathBuf>, theme: Theme) -> Result<Text, String> {
    let app = grind_text::App::new();
    let imported = match &path {
        Some(path) => read(path, |name, bytes| app.open_bytes(name, bytes))?,
        None => None,
    };
    let path = path.filter(|_| imported.is_none());
    let mut text = Text {
        app,
        imported: None,
        path,
        theme,
        dirty: false,
        page: Page {
            status_h: text::geom::STATUS_H,
            dpi: 96,
            ..Page::default()
        },
        flow: Flow::default(),
        across: HashMap::new(),
        caret: START,
        anchor: START,
        goal_x: None,
        fonts: None,
        fonts_dpi: 0,
        dragging: false,
        // On, so that a document opened and immediately rendered shows its caret — and so that
        // two `--render-to` frames of one document are identical, which is what that flag is for.
        caret_on: true,
        banner: None,
        resume: None,
        surrogate: None,
        show_names: false,
        needle: String::new(),
        hover: None,
        pressed: None,
        // Remembered for every text document, as the grid's is for every spreadsheet.
        spelling: grind_spell::preference::load(DocumentKind::Text),
        checked: None,
        speller: None,
    };
    text.check_spelling();
    // The report's sentence on the notice bar, where a state the document is in belongs.
    if let Some(imported) = imported {
        text.imported = Some(PathBuf::from(imported.name));
        text.dirty = true;
        text.say(Some(imported.summary));
    }
    Ok(text)
}

fn opened_sheet(path: Option<PathBuf>, theme: Theme) -> Result<Sheet, String> {
    let app = grind_sheet::App::new();
    let imported = match &path {
        Some(path) => read(path, |name, bytes| app.open_bytes(name, bytes))?,
        None => None,
    };
    let mut sheet = opened_sheet_on(app, path, theme);
    // An imported workbook: no path for Save to write over, unsaved, and the report's sentence
    // on the notice bar, where a state the document is in belongs.
    if let Some(imported) = imported {
        sheet.path = None;
        sheet.imported = Some(PathBuf::from(imported.name));
        sheet.dirty = true;
        sheet.say(Some(imported.summary));
    }
    Ok(sheet)
}

fn opened_sheet_on(app: grind_sheet::App, path: Option<PathBuf>, theme: Theme) -> Sheet {
    let remembered = grind_spell::preference::load(DocumentKind::Spreadsheet);
    let mut sheet = Sheet {
        app,
        tabs: std::cell::RefCell::new(Vec::new()),
        path,
        imported: None,
        sheet: 0,
        geom: GridGeom {
            format_h: draw::STRIP_H,
            strip_h: draw::STRIP_H,
            banner_h: 0.0,
            hint_h: 0.0,
            header_w: draw::HEADER_W,
            header_h: draw::HEADER_H,
            status_h: draw::STATUS_H,
            rows: Sizes::new(draw::ROW_H, MAX_ROWS, Vec::new()),
            cols: Sizes::new(draw::COL_W, MAX_COLS, Vec::new()),
            first_row: 0,
            first_col: 0,
            width: 0.0,
            height: 0.0,
            dpi: 96,
            zoom: 1.0,
        },
        theme,
        selection: Selection::default(),
        drag: None,
        resize: None,
        name_box: HWND::default(),
        name_box_open: false,
        editor: HWND::default(),
        editor_on_bar: false,
        mode: state::Mode::default(),
        dirty: false,
        banner: None,
        needle: String::new(),
        assist: assist::Assist::default(),
        hint: Vec::new(),
        pointing: None,
        formulas: false,
        // **On**, as in the GNOME window, so a formula reads the same in both unless somebody says
        // otherwise. The reading is only shown; opening the cell brings back the text that will be
        // stored.
        friendly: true,
        ui_font: None,
        field_brush: None,
        surrogate: None,
        overlays: grind_sheet::view::Overlays::NONE,
        format_hover: None,
        format_pressed: None,
        chart_selected: None,
        chart_grab: None,
        spelling: remembered,
        spelling_on: match remembered {
            grind_spell::Setting::Off => grind_spell::Setting::Automatic,
            on => on,
        },
        checked: None,
        speller: None,
        settled: false,
        spell_stop: None,
    };
    sheet.check_spelling();
    sheet
}

/// Draw one frame of a document to a `.bmp` and exit — **no window, no compositor, no display**
/// (decision 5). Not a user feature: how custom drawing gets an assertable output, a refactor
/// proved one when the file comes back byte-identical. Everything that could differ between runs
/// is pinned: the size is a constant and the theme the flag's.
pub fn render(
    kind: Option<DocumentKind>,
    path: Option<PathBuf>,
    target: &std::path::Path,
    dark: bool,
) -> Result<(), String> {
    // COM, for `image.rs`'s WIC calls — a picture is measured in `relayout` and drawn in
    // `draw_text_frame`, and this windowless path has no `run`'s own guard to lean on.
    let _com = Com::new();
    // The palette is the **flag's**, never the machine's: two renders on two machines must be the
    // same bytes, and `--dark` is how the dark palette is looked at without a Windows machine set
    // to dark.
    let mode = match dark {
        true => Mode::Dark,
        false => Mode::Light,
    };
    let mut pane = opened(kind, path, Theme::of(mode))?;
    let (w, h) = (f64::from(RENDER_W), f64::from(RENDER_H));
    let dib = Dib::new(RENDER_W, RENDER_H).ok_or("could not make the drawing surface")?;
    match &mut pane {
        Pane::Sheet(sheet) => {
            sheet.relayout(w, h, 96);
            draw_frame(dib.dc(), sheet);
        }
        Pane::Text(text) => {
            text.relayout(w, h, 96);
            draw_text_frame(dib.dc(), text, false);
        }
        // `--render-to shot.bmp` with no file draws the welcome screen — the only way to look at
        // it without a Windows machine.
        Pane::Welcome(state) => {
            state.relayout(w, h, 96);
            draw_welcome_frame(dib.dc(), state);
        }
    }
    std::fs::write(target, dib.bmp()).map_err(|error| format!("{}: {error}", target.display()))
}

/// Open a window on a document and pump messages until it closes. `path` is `None` for a new,
/// empty document of `kind`; `kind` is `None` when nobody named a document, which opens the
/// welcome screen.
pub fn run(kind: Option<DocumentKind>, path: Option<PathBuf>) -> Result<(), String> {
    // Before *any* window exists: per-monitor v2 cannot be set once a window has been created, and
    // asking late fails silently and leaves a system-DPI-aware window, stretched and blurry at
    // 150%. Best-effort, since it also fails when a manifest set an awareness already.
    // SAFETY: no arguments to outlive the call.
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }

    // COM, for as long as there is a window: `IFileDialog` is a COM object and needs an
    // apartment, and this is the one place with a lifetime long enough to be it. An application
    // that never opens a file dialog pays for one `CoInitializeEx`.
    let _com = Com::new();

    let state = Box::new(opened(kind, path, theme::current())?);

    let class = gdi::wide(CLASS_NAME);
    let title = gdi::wide(&state.title());

    // SAFETY: the class name and title buffers outlive the calls; the boxed state is handed to
    // the window and taken back in `WM_NCDESTROY`. `CreateWindowExW` failing leaks nothing,
    // because the box is only released to the window once `WM_NCCREATE` has stored it — and
    // that message arrives before any failure path.
    unsafe {
        let instance = GetModuleHandleW(None).map_err(|error| format!("no module: {error}"))?;
        let wc = WNDCLASSW {
            // `CS_DBLCLKS` is what makes `WM_LBUTTONDBLCLK` arrive at all: without it Windows
            // sends two plain clicks and a grid has no way to tell a double one apart from a
            // fast pair. It is a class style, so it has to be right before the first window.
            style: CS_DBLCLKS,
            lpfnWndProc: Some(wndproc),
            hInstance: instance.into(),
            lpszClassName: PCWSTR(class.as_ptr()),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            // No class brush at all: `WM_ERASEBKGND` is answered below and every pixel is
            // written by the painter, so a brush here would only be a flash of the wrong
            // colour before each frame.
            ..Default::default()
        };
        if RegisterClassW(&wc) == 0 {
            return Err("could not register the window class".into());
        }

        let hwnd = CreateWindowExW(
            Default::default(),
            PCWSTR(class.as_ptr()),
            PCWSTR(title.as_ptr()),
            WS_OVERLAPPEDWINDOW | WS_VSCROLL | WS_HSCROLL,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            None,
            None::<HMENU>,
            Some(instance.into()),
            Some(Box::into_raw(state).cast()),
        )
        .map_err(|error| format!("could not create the window: {error}"))?;

        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = UpdateWindow(hwnd);

        // `GetMessageW` has three answers, not two: positive for a message, zero for `WM_QUIT`,
        // and **-1 for an error** — on a destroyed window, say. `.as_bool()` is true for -1, so
        // the obvious loop spins forever on the one case that most needs to end.
        let mut message = MSG::default();
        while GetMessageW(&mut message, None, 0, 0).0 > 0 {
            // Enter, Escape, Tab and the arrows inside a child `EDIT` never reach its parent: the
            // control eats them, and with no dialog manager nothing turns them into a command. So
            // the pump relays them (`WM_CHILD_KEY`) and asks rather than decides: it reaches no
            // state, and the window answers whether the key was claimed.
            if message.message == WM_KEYDOWN
                && GetParent(message.hwnd).is_ok_and(|parent| parent == hwnd)
                && SendMessageW(
                    hwnd,
                    WM_CHILD_KEY,
                    Some(WPARAM(message.wParam.0)),
                    Some(LPARAM(message.hwnd.0 as isize)),
                )
                .0 != 0
            {
                continue;
            }
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
    Ok(())
}

/// Run `f` with the window's state, if it has any yet.
///
/// The single point where the raw pointer becomes a reference. The `&mut` lives only for the
/// call, which is what makes decision 7's rule checkable by reading one function rather than
/// every handler: **nothing that runs a nested message loop may be called from inside `f`.**
unsafe fn with_pane<T>(hwnd: HWND, f: impl FnOnce(&mut Pane) -> T) -> Option<T> {
    // SAFETY: the caller is a window procedure for `hwnd`, and the slot holds either null or
    // the pointer stored in `WM_NCCREATE`, which is valid until `WM_NCDESTROY` clears it.
    let raw = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut Pane;
    if raw.is_null() {
        return None;
    }
    // SAFETY: exclusive for the duration of this call. See the module comment's point 3.
    Some(f(unsafe { &mut *raw }))
}

/// The same, for a handler that only means anything to the spreadsheet — `None` when the window
/// is showing a text document, which is what makes "this verb belongs to the other pane" a
/// missing answer rather than a wrong one.
unsafe fn with_sheet<T>(hwnd: HWND, f: impl FnOnce(&mut Sheet) -> T) -> Option<T> {
    // SAFETY: the caller's, unchanged — see [`with_pane`].
    unsafe { with_pane(hwnd, |pane| pane.sheet_mut().map(f)) }.flatten()
}

/// And for the word processor's.
unsafe fn with_text<T>(hwnd: HWND, f: impl FnOnce(&mut Text) -> T) -> Option<T> {
    // SAFETY: the caller's, unchanged — see [`with_pane`].
    unsafe { with_pane(hwnd, |pane| pane.text_mut().map(f)) }.flatten()
}

extern "system" fn wndproc(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match message {
        WM_NCCREATE => {
            // SAFETY: `lparam` is the `CREATESTRUCTW` Windows passes for this message, and its
            // `lpCreateParams` is the pointer `run` handed to `CreateWindowExW`.
            unsafe {
                let create = &*(lparam.0 as *const CREATESTRUCTW);
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
                DefWindowProcW(hwnd, message, wparam, lparam)
            }
        }
        WM_CREATE => {
            // SAFETY: the state was stored by `WM_NCCREATE`, which always precedes this.
            unsafe {
                with_pane(hwnd, |pane| {
                    let theme = theme::current();
                    pane.retheme(theme);
                    theme::apply_window_chrome(hwnd, theme);
                    // The modals paint themselves in this palette too, and cannot be handed it
                    // at the call site — `dialog::use_theme` says why — and so does the bar.
                    dialog::use_theme(theme);
                    menubar::set_theme(theme);
                    // Registered here rather than in `opened`: it needs a window to post to, and a
                    // document read before there is one must not arrive as a change the user made
                    // — so no "loading" flag is needed.
                    let observer = Arc::new(Changed(hwnd.0 as isize));
                    match pane {
                        Pane::Sheet(sheet) => sheet.app.set_observer(observer),
                        Pane::Text(text) => text.app.set_observer(observer),
                        // Nothing to observe: no document, so nothing can change under us.
                        Pane::Welcome(_) => {}
                    }
                });
            }
            build_menu(hwnd);
            make_children(hwnd);
            refresh(hwnd);
            LRESULT(0)
        }
        // The menu bar is this shell's own, drawn in the band above the client area that
        // Windows' would have taken (`menubar.rs`): the band is reserved here, painted on every
        // non-client repaint, and is the menu as far as hit-testing goes.
        WM_NCCALCSIZE => menubar::nc_calc_size(hwnd, wparam, lparam),
        WM_NCPAINT => {
            // SAFETY: the default frame first, with the message's own arguments.
            let result = unsafe { DefWindowProcW(hwnd, message, wparam, lparam) };
            menubar::nc_paint(hwnd);
            result
        }
        WM_NCACTIVATE => {
            // SAFETY: as above.
            let result = unsafe { DefWindowProcW(hwnd, message, wparam, lparam) };
            menubar::nc_paint(hwnd);
            result
        }
        // SAFETY (all three): the default answer, with the message's own arguments.
        WM_NCHITTEST => menubar::nc_hit(hwnd, lparam)
            .unwrap_or_else(|| unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }),
        WM_NCMOUSEMOVE => {
            menubar::nc_mouse_move(hwnd, lparam);
            unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
        }
        WM_NCMOUSELEAVE => {
            menubar::nc_mouse_leave(hwnd);
            unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
        }
        WM_NCLBUTTONDOWN if wparam.0 == HTMENU as usize => {
            if let Some(command) = menubar::nc_button_down(hwnd, lparam).and_then(menu::command_for)
            {
                do_command(hwnd, command);
            }
            LRESULT(0)
        }
        // Alt, F10 and Alt+letter, which Windows turns into this whether or not a window has a
        // bar of its own — and the drawn one answers it.
        WM_SYSCOMMAND if (wparam.0 & 0xfff0) == SC_KEYMENU as usize => {
            match menubar::key_menu(hwnd, lparam.0 as u32) {
                Some(picked) => {
                    if let Some(command) = picked.and_then(menu::command_for) {
                        do_command(hwnd, command);
                    }
                    LRESULT(0)
                }
                // SAFETY: Alt+Space, the window menu, handed back.
                None => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
            }
        }
        // Nothing to erase: the painter writes every pixel of the client area onto a back
        // buffer and blits it in one move, so letting the default erase first would show the
        // class brush for a frame. Answering non-zero is the documented way to say "done".
        WM_ERASEBKGND => LRESULT(1),
        WM_SIZE => {
            refresh(hwnd);
            LRESULT(0)
        }
        WM_PAINT => {
            paint(hwnd);
            LRESULT(0)
        }
        WM_VSCROLL => {
            scroll(hwnd, wparam, true);
            LRESULT(0)
        }
        WM_HSCROLL => {
            scroll(hwnd, wparam, false);
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            mouse_move(hwnd, lparam);
            LRESULT(0)
        }
        // The two-headed arrow over a header boundary, and while one is being dragged — the
        // pointer is reset to the class cursor by every `WM_SETCURSOR`, so it is answered here or
        // not at all.
        WM_SETCURSOR if set_resize_cursor(hwnd) => LRESULT(1),
        WM_LBUTTONDOWN => {
            button_down(hwnd, lparam);
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            button_up(hwnd);
            LRESULT(0)
        }
        WM_LBUTTONDBLCLK => {
            double_click(hwnd, lparam);
            LRESULT(0)
        }
        // A right click, **and** Shift+F10 or the Menu key — Windows sends this one message for
        // all three, so the context menus answer the keyboard for free. `lparam`'s point is in
        // *screen* coordinates here, and `(-1, -1)` exactly when the keyboard asked.
        WM_CONTEXTMENU => {
            context_menu(hwnd, lparam);
            LRESULT(0)
        }
        // A menu is about to open, from the bar or a right click (`TrackPopupMenuEx` sends this
        // too). The checks that depend on the selection — the active cell's currency, the strip's
        // toggles — are set here, since `build_menu` runs only when the pane changes.
        WM_INITMENUPOPUP => {
            check_format(hwnd, HMENU(wparam.0 as *mut std::ffi::c_void));
            check_chart_kind(hwnd, HMENU(wparam.0 as *mut std::ffi::c_void));
            LRESULT(0)
        }
        // A character, after the keyboard layout and after the IME — which is why a printable
        // key is decided here and not in `WM_KEYDOWN`. Deciding "is this printable" on a
        // virtual-key code is the bug that makes an accented character unable to start an edit.
        WM_CHAR => match typed_char(hwnd, wparam.0 as u32) {
            true => LRESULT(0),
            // SAFETY: a character this shell does not start an edit with, handed back.
            false => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
        },
        WM_KEYDOWN => match key_down(hwnd, wparam.0 as u32) {
            true => LRESULT(0),
            // SAFETY: a key this shell does not own, handed back with the arguments it came
            // with — which is what leaves Alt+F4 and the system menu working.
            false => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
        },
        // A composing IME is about to show its candidate window — moved to the caret before
        // `DefWindowProcW` draws it. The rest of composing (the list, the committed `WM_CHAR`s) is
        // the default handling; this is the one IME message answered.
        WM_IME_STARTCOMPOSITION => {
            position_ime_composition(hwnd);
            // SAFETY: the arguments this message came with, unchanged.
            unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
        }
        // A key that went to one of the child `EDIT`s, relayed by the pump in `run`.
        WM_CHILD_KEY => LRESULT(isize::from(child_key(
            hwnd,
            wparam.0 as u32,
            HWND(lparam.0 as *mut std::ffi::c_void),
        ))),
        // A menu item (`lparam` is zero) or a notification from a child control (it is not).
        // Win32 puts both through one message and tells them apart by that, which is why the
        // control ids are below `menu::FIRST_ID` rather than merely different.
        WM_COMMAND => {
            let id = (wparam.0 & 0xffff) as u16;
            let code = ((wparam.0 >> 16) & 0xffff) as u32;
            match (lparam.0, code, usize::from(id)) {
                (0, _, _) => {
                    if let Some(command) = menu::command_for(id) {
                        do_command(hwnd, command);
                    }
                }
                // The name box lost the focus without being finished — a click on the grid, or
                // Alt+Tab. Closing without committing is what every address box does.
                (_, EN_KILLFOCUS, ID_NAME_BOX) => close_name_box(hwnd, false),
                // The editor lost it, which is *not* the same answer: a half-typed cell that
                // vanishes because the user clicked elsewhere is lost work, and every
                // spreadsheet commits instead.
                (_, EN_KILLFOCUS, ID_EDITOR) => commit_edit(hwnd, None),
                // An in-cell edit is mirrored on the drawn formula bar as it is typed, so the
                // strip — and only the strip — is repainted per keystroke.
                (_, EN_CHANGE, ID_EDITOR) => editor_changed(hwnd),
                _ => {}
            }
            LRESULT(0)
        }
        // The document changed under us, posted by `Changed`. Every path that edits arrives
        // here, which is what makes the title's `*` impossible to forget.
        WM_DOC_CHANGED => {
            // SAFETY: one borrow, released before the title is set.
            let title = unsafe {
                with_pane(hwnd, |pane| {
                    pane.set_dirty(true);
                    pane.title()
                })
            };
            if let Some(title) = title {
                let wide = gdi::wide(&title);
                // SAFETY: the borrow is released; the buffer is a NUL-terminated local.
                unsafe {
                    let _ = SetWindowTextW(hwnd, PCWSTR(wide.as_ptr()));
                    let _ = InvalidateRect(Some(hwnd), None, false);
                }
            }
            LRESULT(0)
        }
        // The child `EDIT`s are drawn by Windows and not by this shell, so this is the only
        // place they can be made to follow the theme — a white field in a dark window is the
        // one thing that makes a GDI shell look half-finished.
        WM_CTLCOLOREDIT => {
            // SAFETY: `wparam` is the control's `HDC` for this message, live for its duration.
            // The brush is owned by the state and outlives every paint that uses it, which is
            // what this message requires of the value it is given back.
            let brush = unsafe {
                let dc = HDC(wparam.0 as *mut std::ffi::c_void);
                with_sheet(hwnd, |state| {
                    SetBkMode(dc, OPAQUE);
                    SetTextColor(dc, COLORREF(state.theme.text.colorref()));
                    SetBkColor(dc, COLORREF(state.theme.card.colorref()));
                    let field = state.theme.card;
                    state
                        .field_brush
                        .get_or_insert_with(|| Brush::solid(field))
                        .handle()
                })
            };
            match brush {
                Some(brush) => LRESULT(brush.0 as isize),
                // SAFETY: no state yet, so the default answer with the arguments it was given.
                None => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
            }
        }
        // The X, Alt+F4 and Exit all arrive here, which makes them one verb — and this is the last
        // point where "cancel" is still an answer.
        WM_CLOSE => {
            close(hwnd);
            LRESULT(0)
        }
        WM_MOUSEWHEEL => {
            wheel(hwnd, wparam);
            LRESULT(0)
        }
        // The system caret exists only while this window has focus — `CreateCaret` is per-thread,
        // and a dialog or another application taking focus must not find one left behind.
        WM_SETFOCUS => {
            if is_text(hwnd) {
                place_system_caret(hwnd);
            }
            LRESULT(0)
        }
        WM_KILLFOCUS => {
            if is_text(hwnd) {
                // SAFETY: paired with `place_system_caret`'s `CreateCaret`; destroying with none
                // showing is documented as harmless, which covers the grid's own case for free.
                unsafe {
                    let _ = DestroyCaret();
                }
            }
            LRESULT(0)
        }
        // The user changed the theme while the window was open. `WM_SETTINGCHANGE` is sent for
        // many settings, so the registry is re-read rather than the message trusted.
        WM_SETTINGCHANGE => {
            // SAFETY: one borrow, released before the repaint is asked for; nothing inside runs
            // a nested message loop.
            unsafe {
                with_pane(hwnd, |pane| {
                    let theme = theme::current();
                    // The cached brush is the old palette's; `retheme` drops it, and the next
                    // `WM_CTLCOLOREDIT` makes one in the new one.
                    pane.retheme(theme);
                    theme::apply_window_chrome(hwnd, theme);
                    // The modals paint themselves in this palette too, and cannot be handed it
                    // at the call site — `dialog::use_theme` says why — and so does the bar.
                    dialog::use_theme(theme);
                    menubar::set_theme(theme);
                });
                menubar::nc_paint(hwnd);
                let _ = InvalidateRect(Some(hwnd), None, false);
                DefWindowProcW(hwnd, message, wparam, lparam)
            }
        }
        // The window has moved to a monitor with a different scaling. Windows suggests a
        // rectangle that keeps it the same *physical* size; taking it is what makes the drag
        // between two monitors look like one continuous window rather than a jump.
        WM_DPICHANGED => {
            // SAFETY: `lparam` is the suggested `RECT` for this message.
            unsafe {
                let suggested = &*(lparam.0 as *const windows::Win32::Foundation::RECT);
                let _ = SetWindowPos(
                    hwnd,
                    None,
                    suggested.left,
                    suggested.top,
                    suggested.right - suggested.left,
                    suggested.bottom - suggested.top,
                    SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
                );
            }
            refresh(hwnd);
            LRESULT(0)
        }
        WM_DESTROY => {
            // SAFETY: ends the message loop in `run`; nothing is freed here.
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        // The last message this window receives, so the box is taken back here. The slot is zeroed
        // first, so a message arriving from within the drop finds no state rather than a dangling
        // pointer.
        WM_NCDESTROY => {
            // SAFETY: the pointer was made by `Box::into_raw` in `run` and has not been freed;
            // reconstituting it exactly once is what frees it.
            unsafe {
                let raw = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Pane;
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                if !raw.is_null() {
                    drop(Box::from_raw(raw));
                }
                DefWindowProcW(hwnd, message, wparam, lparam)
            }
        }
        // SAFETY: the default handler, with the arguments it was given.
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

/// Recompute everything derived from the document and the client area, then repaint — one
/// function, since a different sheet has different tracks, which move the active cell, which moves
/// the child control on it, and each changes the frame and the title. Every path touching the
/// document ends here.
fn refresh(hwnd: HWND) {
    if is_welcome(hwnd) {
        welcome_refresh(hwnd);
        return;
    }
    if is_text(hwnd) {
        text_refresh(hwnd);
        return;
    }
    let rect = gdi::client_rect(hwnd);
    // SAFETY: `hwnd` is this window's; `GetDpiForWindow` needs nothing else.
    let dpi = unsafe { GetDpiForWindow(hwnd) }.max(96);
    // SAFETY: no nested loop inside.
    let after = unsafe {
        with_sheet(hwnd, |state| {
            state.reconsider_spelling();
            state.relayout(
                f64::from(rect.right - rect.left),
                f64::from(rect.bottom - rect.top),
                dpi,
            );
            // A window that grew or changed monitor may now show the active cell where it did
            // not, and — more to the point — may not show it any more.
            state.reveal();
            sync_scrollbars(hwnd, state);
            // At most one child is ever up: the name box and the editor are opened by the same
            // keys, and each closes the other on its way in.
            if state.name_box_open {
                Some((state.name_box, state.geom.name_box_rect()))
            } else if state.mode.is_editing() {
                Some((state.editor, state.editor_rect()))
            } else {
                None
            }
        })
    };
    // The title is the *window's* rather than the pane's, so it is asked for separately — one more
    // borrow, taken and released like every other.
    // SAFETY: nothing inside dispatches.
    let title = unsafe { with_pane(hwnd, |pane| pane.title()) };
    let (Some(child), Some(title)) = (after, title) else {
        return;
    };
    // SAFETY: the borrow above is released. `MoveWindow` on a visible child and `SetWindowTextW`
    // both dispatch messages, so they belong out here. The title is compared first, since this
    // runs on every `WM_SIZE` and rewriting the caption every pixel of a resize makes it flicker.
    unsafe {
        if let Some((edit, rect)) = child {
            move_to(edit, rect, true);
        }
        if window_text(hwnd) != title {
            let wide = gdi::wide(&title);
            let _ = SetWindowTextW(hwnd, PCWSTR(wide.as_ptr()));
        }
        let _ = InvalidateRect(Some(hwnd), None, false);
    }
}

/// Put a child control on one of the geometry's rectangles — the one place `Rect`'s floats become
/// Win32's integers.
fn move_to(child: HWND, rect: Rect, repaint: bool) {
    // SAFETY: `child` is one of this window's controls and outlives the call.
    unsafe {
        let _ = MoveWindow(
            child,
            rect.x.round() as i32,
            rect.y.round() as i32,
            rect.w.round() as i32,
            rect.h.round() as i32,
            repaint,
        );
    }
}

/// The menu bar, built from [`menu::MENUS`] (decision 4): a command's id comes from its position,
/// so two items cannot share a `WM_COMMAND` id, and reachability is checked on Linux.
///
/// Every item the pane has no answer for (`menu::applies_to`) is **left out**, via
/// `menu::items_for`/`menu::menu_has_items`, rather than greyed: greying left the sheet's many
/// verbs visible and dead over a text document. A menu left empty — `&Sheet`/`&Data` on the text
/// pane — is not put in the bar. Rebuilt when the pane changes (`adopt`), not the selection; a
/// stale accelerator for a command with no item still reaches `do_command`, whose per-pane no-ops
/// make that safe.
fn build_menu(hwnd: HWND) {
    // SAFETY: one borrow, for the kind and the two overlay checkmarks; nothing inside dispatches.
    // The role overlay has no meaning on the text pane (`CellRole` is the grid's alone), so it
    // reads `false` there rather than a second flag nothing ever sets.
    let (surface, roles_on, names_on, friendly_on, formulas_on, spelling_on) = unsafe {
        with_pane(hwnd, |pane| {
            let surface = pane.surface();
            match pane {
                Pane::Sheet(sheet) => (
                    surface,
                    sheet.overlays.roles,
                    sheet.overlays.names,
                    sheet.friendly,
                    sheet.formulas,
                    sheet.spelling != grind_spell::Setting::Off,
                ),
                Pane::Text(text) => (
                    surface,
                    false,
                    text.show_names,
                    false,
                    false,
                    text.spelling != grind_spell::Setting::Off,
                ),
                // No document, so none of the checkmarks means anything — and `items_for`
                // leaves every menu they live in out of the bar anyway.
                Pane::Welcome(_) => (surface, false, false, false, false, false),
            }
        })
    }
    .unwrap_or((menu::Surface::Welcome, false, false, false, false, false));
    // SAFETY: every label buffer outlives the `AppendMenuW` that reads it — Windows copies the
    // string — and the bar belongs to the window from `SetMenu` until it is destroyed with it.
    unsafe {
        let Ok(bar) = CreateMenu() else { return };
        let mut titles = Vec::new();
        let mut popups = Vec::new();
        for menu in menu::MENUS {
            // A menu with nothing this pane answers to — `Sheet`/`Data` on the text pane — is
            // left out of the bar entirely rather than added with an
            // empty popup under its title.
            if !menu::menu_has_items(menu, surface) {
                continue;
            }
            let Ok(popup) = CreatePopupMenu() else {
                continue;
            };
            for item in menu::items_for(menu, surface) {
                match item {
                    Item::Separator => {
                        let _ = AppendMenuW(popup, MF_SEPARATOR, 0, PCWSTR::null());
                    }
                    Item::Verb { command, label } => {
                        let label = gdi::wide(label);
                        let _ = AppendMenuW(
                            popup,
                            MF_STRING,
                            usize::from(command.id()),
                            PCWSTR(label.as_ptr()),
                        );
                        // The two overlays and the friendly bar are this bar's checkable items,
                        // each read once above.
                        let checked = match command {
                            Command::ToggleRoles => Some(roles_on),
                            Command::ToggleNames => Some(names_on),
                            Command::ToggleFriendly => Some(friendly_on),
                            Command::ToggleFormulas => Some(formulas_on),
                            Command::ToggleSpelling => Some(spelling_on),
                            _ => None,
                        };
                        if let Some(checked) = checked {
                            let flag = match checked {
                                true => MF_CHECKED,
                                false => MF_UNCHECKED,
                            };
                            let _ = CheckMenuItem(
                                popup,
                                u32::from(command.id()),
                                (MF_BYCOMMAND | flag).0,
                            );
                        }
                    }
                }
            }
            let title = gdi::wide(menu.title);
            let _ = AppendMenuW(bar, MF_POPUP, popup.0 as usize, PCWSTR(title.as_ptr()));
            titles.push(menu.title);
            popups.push(popup);
        }
        // Kept rather than attached: attaching it is what would bring Windows' own bar back, and
        // the bar is drawn by `menubar.rs` in the band Windows' own would have taken.
        menubar::install(hwnd, bar, titles, popups);
    }
}

/// Whether a `WM_CONTEXTMENU` is about a chart — under the pointer (which it then selects, as a
/// right click on a cell moves the selection) or, from the keyboard, the one already selected.
fn context_on_chart(hwnd: HWND, lparam: LPARAM) -> bool {
    let (x, y) = point(lparam);
    let mut at = POINT {
        x: x.round() as i32,
        y: y.round() as i32,
    };
    let keyboard = (x, y) == (-1.0, -1.0);
    if !keyboard {
        // SAFETY: `hwnd` is this window's and `at` is a live local.
        unsafe {
            let _ = ScreenToClient(hwnd, &mut at);
        }
    }
    // SAFETY: one borrow; arithmetic.
    let on = unsafe {
        with_sheet(hwnd, |state| {
            if keyboard {
                return state.chart_now().is_some();
            }
            let charts = state.app.charts(state.sheet).unwrap_or_default();
            let (x, y) = (f64::from(at.x), f64::from(at.y));
            match crate::sheet::chart::hit(&charts, &state.geom, state.chart_now(), x, y) {
                Some((index, _)) => {
                    state.chart_selected = Some((state.sheet, index));
                    true
                }
                None => false,
            }
        })
    } == Some(true);
    if on {
        // SAFETY: no borrow held.
        unsafe {
            let _ = InvalidateRect(Some(hwnd), None, false);
            let _ = UpdateWindow(hwnd);
        }
    }
    on
}

/// The context menus: the grid's clipboard verbs over cells and headers, the text pane's plus
/// the strip's five toggles — a fixed list of real [`Command`]s from [`menu::MENUS`], so a
/// click here reaches the handler a click on the bar would.
fn context_menu(hwnd: HWND, lparam: LPARAM) {
    // The welcome screen's own: the three cards, which is what a right click on a screen made of
    // three choices can usefully offer. Clipboard verbs would be items over nothing.
    let on_tab = context_on_tab(hwnd, lparam);
    let on_chart = !on_tab && !is_welcome(hwnd) && !is_text(hwnd) && context_on_chart(hwnd, lparam);
    let commands: &[Command] = if is_welcome(hwnd) {
        &[Command::NewSheet, Command::NewText, Command::Open]
    } else if on_chart {
        // A chart's own menu, over the chart `context_on_chart` has just selected — the three
        // verbs every window offers there, each of which has its key on a selected chart too.
        &[
            Command::ChartToBar,
            Command::ChartToLine,
            Command::ChartToPie,
            Command::RestyleChart,
            Command::MoveChart,
            Command::DeleteChart,
        ]
    } else if on_tab {
        // A tab's own menu: the three verbs the Sheet menu already has, over the tab under the
        // pointer — which `context_on_tab` has just brought to the front, so "this sheet" is
        // the one that was clicked rather than whichever happened to be showing.
        &[
            Command::SheetRename,
            Command::SheetDelete,
            Command::SheetAdd,
        ]
    } else {
        match is_text(hwnd) {
            true => &[
                Command::Cut,
                Command::Copy,
                Command::Paste,
                Command::Bold,
                Command::Italic,
                Command::Underline,
                Command::Strike,
                Command::Code,
                Command::ClearFormatting,
            ],
            false => &[
                Command::Cut,
                Command::Copy,
                Command::Paste,
                Command::ClearCells,
                Command::NumberFormat,
                Command::CurrencyEuro,
                Command::CurrencyDollar,
                Command::CurrencyPound,
                Command::ClearFormatting,
            ],
        }
    };
    // `(-1, -1)` is Windows' own spelling of "the keyboard asked, not the mouse" — Shift+F10 or
    // the Menu key carry no position, so the menu opens over a point of this window's choosing
    // rather than the pointer's.
    let (x, y) = point(lparam);
    let at = match (x, y) {
        (-1.0, -1.0) => context_anchor(hwnd),
        _ => POINT {
            x: x.round() as i32,
            y: y.round() as i32,
        },
    };
    // Over a misspelt word the text pane's menu leads with what it might have been — the word
    // under the pointer, or for the keyboard the one the caret is in.
    if is_text(hwnd) && !is_welcome(hwnd) {
        let keyboard = (x, y) == (-1.0, -1.0);
        let mut client = at;
        // SAFETY: `hwnd` is this window's and `client` is a live local.
        unsafe {
            let _ = ScreenToClient(hwnd, &mut client);
        }
        // SAFETY: one borrow, released before the popup.
        let wrong = unsafe {
            with_text(hwnd, |text| {
                text.checked?;
                let caret = match keyboard {
                    true => text.caret,
                    false => text.caret_at(f64::from(client.x), f64::from(client.y))?,
                };
                text.misspelt_at(caret)
            })
        }
        .flatten();
        // Over a picture the menu offers its alt text, with the caret put beside it first so the
        // verb means *this* picture — the way a right click elsewhere moves the selection.
        // SAFETY: one borrow, released before the popup.
        let picture = unsafe {
            with_text(hwnd, |text| {
                let caret = match keyboard {
                    true => text.caret,
                    false => text.caret_at(f64::from(client.x), f64::from(client.y))?,
                };
                let viewport = text.app.get_viewport(caret.block..caret.block + 1);
                let (offset, _) =
                    grind_text::picture::at(viewport.get(caret.block)?, caret.offset)?;
                let at = Caret {
                    block: caret.block,
                    offset,
                };
                text.place(at, false);
                Some(at)
            })
        }
        .flatten();
        let with_alt: Vec<Command> = match picture {
            Some(_) => std::iter::once(Command::AltText)
                .chain(commands.iter().copied())
                .collect(),
            None => commands.to_vec(),
        };
        spelling_popup(hwnd, at, wrong, &with_alt);
        return;
    }
    // Over a cell with a misspelt word in it the grid's menu leads the same way — the cell under
    // the pointer, or for the keyboard the active one.
    if !is_welcome(hwnd) && !on_tab && !on_chart {
        let keyboard = (x, y) == (-1.0, -1.0);
        let mut client = at;
        // SAFETY: `hwnd` is this window's and `client` is a live local.
        unsafe {
            let _ = ScreenToClient(hwnd, &mut client);
        }
        // SAFETY: one borrow, released before the popup.
        let wrong = unsafe {
            with_sheet(hwnd, |state| {
                state.checked?;
                let pos = match keyboard {
                    true => state.selection.active,
                    false => match state.geom.hit(f64::from(client.x), f64::from(client.y)) {
                        crate::sheet::geom::Hit::Cell { row, col } => Pos::new(row, col),
                        _ => return None,
                    },
                };
                state
                    .app
                    .misspellings(Some(state.sheet), Some((pos, pos)))
                    .ok()?
                    .into_iter()
                    .next()
            })
        }
        .flatten();
        if wrong.is_some() {
            sheet_spelling_popup(hwnd, at, wrong, commands);
            return;
        }
    }
    // SAFETY: the popup is built and destroyed within this call, and `TrackPopupMenuEx` is the
    // one nested message loop in it — decision 7's rule, and nothing is borrowed across it.
    let picked = unsafe {
        let Ok(popup) = CreatePopupMenu() else {
            return;
        };
        for command in commands {
            // The currencies are a group of their own, as they are in the Format menu.
            if command.currency() == Some(0) {
                let _ = AppendMenuW(popup, MF_SEPARATOR, 0, PCWSTR::null());
            }
            if let Some(label) = menu::label_for(*command) {
                let label = gdi::wide(label);
                let _ = AppendMenuW(
                    popup,
                    MF_STRING,
                    usize::from(command.id()),
                    PCWSTR(label.as_ptr()),
                );
            }
        }
        let result = TrackPopupMenuEx(
            popup,
            (TPM_RETURNCMD | TPM_RIGHTBUTTON).0,
            at.x,
            at.y,
            hwnd,
            None,
        );
        let _ = DestroyMenu(popup);
        result
    };
    if let Some(command) = menu::command_for(picked.0 as u16) {
        do_command(hwnd, command);
    }
}

/// Where a keyboard-invoked context menu opens: the middle of the client area, in screen
/// coordinates — simpler than anchoring on the caret or the active cell, and the menu still
/// reaches every verb, which is what Shift+F10 promises.
fn context_anchor(hwnd: HWND) -> POINT {
    let rect = gdi::client_rect(hwnd);
    let mut point = POINT {
        x: (rect.right - rect.left) / 2,
        y: (rect.bottom - rect.top) / 2,
    };
    // SAFETY: `hwnd` is this window's and `point` is a live local.
    unsafe {
        let _ = ClientToScreen(hwnd, &mut point);
    }
    point
}

/// Create the two child `EDIT`s and their face — hidden from birth and shown on demand, so the
/// strip is drawn the rest of the time and appears in a `--render-to` frame. They hold an address
/// on its way to `status::locate` and a cell's text on its way to `App::enter`, never the document
/// (decision 2).
fn make_children(hwnd: HWND) {
    // The text pane has no child controls — it draws its own caret and holds its own keystrokes —
    // so a window showing one creates none. A window that *becomes* a spreadsheet comes back
    // through here, so this is idempotent.
    // SAFETY: one borrow, nothing inside dispatches.
    match unsafe { with_sheet(hwnd, |state| state.editor.is_invalid()) } {
        Some(true) => {}
        _ => return,
    }
    let class = gdi::wide("EDIT");
    // SAFETY: the class name outlives both calls; the controls are destroyed with their parent,
    // and the font they are given is owned by the state, which outlives both.
    unsafe {
        let dpi = GetDpiForWindow(hwnd).max(96);
        // The strip's own size, not a cell's: the two `EDIT`s stand in for the name box and the
        // formula bar, which are drawn at `theme::text::BODY`, and a control whose text is a
        // pixel smaller than the read-out it replaces makes the whole strip twitch on every edit.
        let font = Font::new(face(), scale(theme::text::BODY, dpi).round() as i32, false);
        let make = |id: usize| -> HWND {
            let Ok(edit) = CreateWindowExW(
                Default::default(),
                PCWSTR(class.as_ptr()),
                PCWSTR::null(),
                windows::Win32::UI::WindowsAndMessaging::WINDOW_STYLE(ES_AUTOHSCROLL as u32)
                    | WS_CHILD,
                0,
                0,
                0,
                0,
                Some(hwnd),
                Some(HMENU(id as *mut std::ffi::c_void)),
                None,
                None,
            ) else {
                return HWND::default();
            };
            SendMessageW(
                edit,
                WM_SETFONT,
                Some(WPARAM(font.handle().0 as usize)),
                Some(LPARAM(1)),
            );
            edit
        };
        let (name_box, editor) = (make(ID_NAME_BOX), make(ID_EDITOR));
        with_sheet(hwnd, |state| {
            state.name_box = name_box;
            state.editor = editor;
            // Kept alive here: `WM_SETFONT` does not copy the handle, so deleting the font
            // while a control still refers to it is the leak-shaped bug in reverse.
            state.ui_font = Some(font);
        });
    }
}

/// Tell Windows where the thumbs are and how big. The range is in **tracks, not pixels**: a
/// `SCROLLINFO` is `i32`, and this sheet is twenty million pixels but a million rows.
fn sync_scrollbars(hwnd: HWND, state: &Sheet) {
    let set = |bar, max: u32, pos: u32, page: i64| {
        let info = SCROLLINFO {
            cbSize: u32::try_from(std::mem::size_of::<SCROLLINFO>()).expect("small"),
            fMask: SCROLLINFO_MASK(SIF_RANGE.0 | SIF_PAGE.0 | SIF_POS.0),
            nMin: 0,
            // The maximum is the *last first track*, so the thumb reaches the end exactly when
            // the last row does — with `nPage` on top, which is how Win32 spells "the thumb has
            // a size" and is why the range is the whole sheet rather than the reachable part.
            nMax: i32::try_from(max.saturating_add(page.max(1) as u32 - 1)).unwrap_or(i32::MAX),
            nPage: u32::try_from(page.max(1)).unwrap_or(u32::MAX),
            nPos: i32::try_from(pos).unwrap_or(i32::MAX),
            nTrackPos: 0,
        };
        // SAFETY: `info` is a live, fully initialised local read for the length of the call.
        unsafe {
            SetScrollInfo(hwnd, bar, &info, true);
        }
    };
    set(
        SB_VERT,
        state.geom.max_first_row(),
        state.geom.first_row,
        state.geom.page_rows(),
    );
    set(
        SB_HORZ,
        state.geom.max_first_col(),
        state.geom.first_col,
        state.geom.page_cols(),
    );
}

/// One scrollbar message, on whichever axis.
fn scroll(hwnd: HWND, wparam: WPARAM, vertical: bool) {
    if is_text(hwnd) {
        text_scroll(hwnd, wparam, vertical);
        return;
    }
    let code = (wparam.0 & 0xffff) as i32;
    let thumb = ((wparam.0 >> 16) & 0xffff) as u32;
    // SAFETY: no nested loop inside.
    unsafe {
        with_sheet(hwnd, |state| {
            let g = &mut state.geom;
            let (at, page) = match vertical {
                true => (g.first_row, g.page_rows()),
                false => (g.first_col, g.page_cols()),
            };
            // Every arm becomes a signed number of tracks, so that the clamping and the
            // hidden-track stepping happen in one portable place rather than in five arms.
            let delta = match scroll_code(code) {
                SB_LINEUP => -1,
                SB_LINEDOWN => 1,
                SB_PAGEUP => -page,
                SB_PAGEDOWN => page,
                SB_TOP => -i64::from(MAX_ROWS),
                SB_BOTTOM => i64::from(MAX_ROWS),
                // Dragging reports an absolute position rather than a delta, and both codes are
                // handled so the view follows the thumb live instead of jumping on release.
                SB_THUMBPOSITION | SB_THUMBTRACK => i64::from(thumb) - i64::from(at),
                _ => return,
            };
            match vertical {
                true => g.scroll_rows(delta),
                false => g.scroll_cols(delta),
            }
            sync_scrollbars(hwnd, state);
        });
        let _ = InvalidateRect(Some(hwnd), None, false);
    }
}

/// `SB_*` constants are `u32` in the `windows` crate and the message packs an `i32`; this names
/// the conversion instead of scattering casts through the match above.
fn scroll_code(code: i32) -> windows::Win32::UI::WindowsAndMessaging::SCROLLBAR_COMMAND {
    windows::Win32::UI::WindowsAndMessaging::SCROLLBAR_COMMAND(code)
}

/// The wheel, by the user's own `SPI_GETWHEELSCROLLLINES` — one line or a screenful, as the mouse
/// control panel set it. `WHEEL_DELTA` is one notch; a precision wheel's fractions are not
/// carried, so every notch moves at least one row.
fn wheel(hwnd: HWND, wparam: WPARAM) {
    let notches = f64::from(((wparam.0 >> 16) & 0xffff) as i16) / f64::from(WHEEL_DELTA);
    let mut lines = 3u32;
    // SAFETY: the out-parameter is a live local of the size the flag implies.
    unsafe {
        let _ = SystemParametersInfoW(
            SPI_GETWHEELSCROLLLINES,
            0,
            Some(std::ptr::from_mut(&mut lines).cast()),
            Default::default(),
        );
    }
    if is_text(hwnd) {
        text_wheel(hwnd, notches, lines);
        return;
    }
    // Ctrl+wheel zooms the grid, as it does in every spreadsheet.
    if mods().ctrl {
        zoom_by(hwnd, if notches > 0.0 { 1.25 } else { 0.8 });
        return;
    }
    // Zero means "do not scroll", which is a real setting. `WHEEL_PAGESCROLL` (0xFFFFFFFF) means a
    // screenful, and is answered with the page rather than with 4 294 967 295 rows.
    // SAFETY: no nested loop inside.
    unsafe {
        with_sheet(hwnd, |state| {
            let page = state.geom.page_rows();
            let lines = match lines {
                0 => return,
                u32::MAX => page,
                n => i64::from(n),
            };
            // Down on the wheel is a *negative* notch count and a *positive* row delta.
            let delta = -(notches * lines as f64).round() as i64;
            state.geom.scroll_rows(delta);
            sync_scrollbars(hwnd, state);
        });
        let _ = InvalidateRect(Some(hwnd), None, false);
    }
}

/// A drag in progress, following the pointer. Nothing happens with no button down: a plain mouse
/// move arrives on every pixel and must cost nothing.
fn mouse_move(hwnd: HWND, lparam: LPARAM) {
    let (x, y) = point(lparam);
    if is_welcome(hwnd) {
        welcome_mouse_move(hwnd, x, y);
        return;
    }
    if is_text(hwnd) {
        text_mouse_move(hwnd, x, y);
        return;
    }
    // SAFETY: no nested loop inside.
    let moved = unsafe {
        with_sheet(hwnd, |state| {
            // A held chart follows the pointer; nothing is written until the button comes up.
            if let Some(grab) = state.chart_grab.as_mut() {
                grab.follow(x, y, mods().shift, &state.geom);
                return true;
            }
            // The format strip's hover, repainted only when it *changes*. `WM_MOUSELEAVE` is not
            // tracked: a pointer leaving the window through the strip is answered by the next move
            // anywhere. A track being sized follows the pointer, never smaller than a few pixels,
            // so it cannot be dragged out of reach (hiding is its own verb).
            if let Some((edge, from, size)) = state.resize {
                let min = scale(8.0, state.geom.dpi);
                match edge {
                    Edge::Col(col) => {
                        let wide = (size + x - from).max(min);
                        state.geom.cols = state.geom.cols.with(col, wide);
                    }
                    Edge::Row(row) => {
                        let tall = (size + y - from).max(min);
                        state.geom.rows = state.geom.rows.with(row, tall);
                    }
                }
                return true;
            }
            let hover = state.geom.format_hit(x, y);
            if hover != state.format_hover {
                state.format_hover = hover;
                if state.drag.is_none() {
                    return true;
                }
            }
            if state.drag.is_none() {
                return false;
            }
            let before = state.selection;
            state.drag_to(state.geom.hit(x, y));
            let moved = state.selection != before;
            if moved {
                sync_scrollbars(hwnd, state);
            }
            moved
        })
    };
    if moved == Some(true) {
        // SAFETY: the borrow above has been released.
        unsafe {
            let _ = InvalidateRect(Some(hwnd), None, false);
        }
    }
}

/// Show the resize cursor when the pointer is on a header boundary or a track is being sized.
/// `true` when it did, and `WM_SETCURSOR` is then answered.
fn set_resize_cursor(hwnd: HWND) -> bool {
    if is_welcome(hwnd) || is_text(hwnd) {
        return false;
    }
    let mut at = windows::Win32::Foundation::POINT::default();
    // SAFETY: `at` is a live local; the point is converted into this window's client space.
    unsafe {
        if GetCursorPos(&mut at).is_err() {
            return false;
        }
        let _ = ScreenToClient(hwnd, &mut at);
    }
    // SAFETY: one borrow; arithmetic.
    let edge = unsafe {
        with_sheet(hwnd, |state| {
            state
                .resize
                .map(|(edge, ..)| edge)
                .or_else(|| state.geom.edge_at(f64::from(at.x), f64::from(at.y)))
        })
    }
    .flatten();
    let cursor = match edge {
        Some(Edge::Col(_)) => IDC_SIZEWE,
        Some(Edge::Row(_)) => IDC_SIZENS,
        // Over a chart, or holding one: the pointer says what a drag there would do.
        None => {
            // SAFETY: one borrow; arithmetic.
            let grip = unsafe {
                with_sheet(hwnd, |state| match state.chart_grab {
                    Some(grab) => Some(grab.grip),
                    None => {
                        let charts = state.app.charts(state.sheet).unwrap_or_default();
                        let (x, y) = (f64::from(at.x), f64::from(at.y));
                        crate::sheet::chart::hit(&charts, &state.geom, state.chart_now(), x, y)
                            .map(|(_, grip)| grip)
                    }
                })
            }
            .flatten();
            match grip.map(|grip| grip.cursor()) {
                Some("move") => IDC_SIZEALL,
                Some("ns-resize") => IDC_SIZENS,
                Some("ew-resize") => IDC_SIZEWE,
                Some("nesw-resize") => IDC_SIZENESW,
                Some(_) => IDC_SIZENWSE,
                None => return false,
            }
        }
    };
    // SAFETY: a system cursor, which is never freed.
    unsafe {
        if let Ok(cursor) = LoadCursorW(None, cursor) {
            SetCursor(Some(cursor));
        }
    }
    true
}

/// Where a mouse message happened, in client space — two *signed* 16-bit numbers, so dragging
/// above or left of the client gives a negative coordinate, not a huge one.
fn point(lparam: LPARAM) -> (f64, f64) {
    (
        f64::from((lparam.0 & 0xffff) as i16),
        f64::from(((lparam.0 >> 16) & 0xffff) as i16),
    )
}

/// Which modifier keys are down, asked of the keyboard since `WM_KEYDOWN` carries only the key.
/// `GetKeyState`'s high bit is "down"; its low bit is the toggle state, which makes Caps Lock look
/// pressed to a test for non-zero.
fn mods() -> keymap::Mods {
    // SAFETY: no arguments, and the answer is a copy.
    let down = |vk: windows::Win32::UI::Input::KeyboardAndMouse::VIRTUAL_KEY| unsafe {
        GetKeyState(i32::from(vk.0)) < 0
    };
    keymap::Mods {
        ctrl: down(VK_CONTROL),
        shift: down(VK_SHIFT),
        alt: down(VK_MENU),
    }
}

/// One keystroke from the window itself; `false` hands it back to `DefWindowProc`, leaving Alt+F4
/// and the system menu working.
fn key_down(hwnd: HWND, vk: u32) -> bool {
    if is_welcome(hwnd) {
        return welcome_key(hwnd, vk);
    }
    if is_text(hwnd) {
        return text_key(hwnd, vk);
    }
    // SAFETY: no nested loop inside.
    let mode = unsafe { with_sheet(hwnd, |state| state.mode) }.unwrap_or_default();
    if !mode.is_editing() && chart_key(hwnd, vk) {
        return true;
    }
    on_key(hwnd, mode, vk)
}

/// A press at `(x, y)` on the grid, asked of the charts first: on one it is selected and held —
/// `Some` — and anywhere else a selected chart is let go of.
fn chart_press(state: &mut Sheet, x: f64, y: f64) -> Option<()> {
    let charts = state.app.charts(state.sheet).unwrap_or_default();
    let selected = state.chart_now();
    let Some((index, grip)) = crate::sheet::chart::hit(&charts, &state.geom, selected, x, y) else {
        state.chart_selected = None;
        return None;
    };
    let start = crate::sheet::chart::frames(&charts, &state.geom)
        .get(index)
        .copied()
        .flatten()?;
    state.chart_selected = Some((state.sheet, index));
    state.chart_grab = Some(crate::sheet::chart::Grab {
        index,
        grip,
        start,
        from: (x, y),
        now: start,
        was_selected: selected == Some(index),
    });
    state.drag = None;
    Some(())
}

/// A key while a chart is selected (`grind_sheet::chart_frame::Key`): Delete and Backspace delete
/// it, Escape lets go, the arrows nudge it (Shift for a large step), Return changes it. Any other
/// key lets go of it and means what it always means — `false`.
fn chart_key(hwnd: HWND, vk: u32) -> bool {
    use grind_sheet::chart_frame::{self, Key};
    // SAFETY: one borrow, for one index.
    let Some(index) = (unsafe { with_sheet(hwnd, |state| state.chart_now()) }).flatten() else {
        return false;
    };
    let held = mods();
    let key = match keymap::key_for(vk) {
        keymap::Key::Delete | keymap::Key::Backspace => Key::Delete,
        keymap::Key::Escape => Key::Deselect,
        keymap::Key::Left => chart_frame::nudge(-1, 0, held.shift),
        keymap::Key::Right => chart_frame::nudge(1, 0, held.shift),
        keymap::Key::Up => chart_frame::nudge(0, -1, held.shift),
        keymap::Key::Down => chart_frame::nudge(0, 1, held.shift),
        keymap::Key::Return => {
            restyle_chart(hwnd);
            return true;
        }
        // A modifier on its own is the start of a chord, not a reason to let go.
        _ if matches!(vk, 0x10..=0x12) => return false,
        _ => {
            // SAFETY: one borrow, one field.
            unsafe { with_sheet(hwnd, |state| state.chart_selected = None) };
            // SAFETY: no borrow held.
            unsafe {
                let _ = InvalidateRect(Some(hwnd), None, false);
            }
            return false;
        }
    };
    match key {
        Key::Delete => delete_chart(hwnd),
        Key::Deselect => {
            // SAFETY: one borrow, one field.
            unsafe { with_sheet(hwnd, |state| state.chart_selected = None) };
            refresh(hwnd);
        }
        nudge @ Key::Nudge(..) => {
            // SAFETY: one borrow; the write notifies and the observer posts rather than sends.
            unsafe {
                with_sheet(hwnd, |state| {
                    let charts = state.app.charts(state.sheet).unwrap_or_default();
                    let Some(Some(frame)) = crate::sheet::chart::frames(&charts, &state.geom)
                        .get(index)
                        .copied()
                    else {
                        return;
                    };
                    let moved = crate::sheet::chart::nudged(frame, nudge, &state.geom);
                    let [x, y, w, h] = crate::sheet::chart::lengths(moved, &state.geom);
                    if let Err(error) = state.app.reshape_chart(state.sheet, index, &x, &y, &w, &h)
                    {
                        state.say(Some(error.to_string()));
                    }
                })
            };
            refresh(hwnd);
        }
    }
    true
}

/// One keystroke relayed by the pump from a child control; `true` means the shell claimed it and
/// the control must not see it. The name box has two keys and no mode; the editor has
/// [`state::on_key`]'s whole table.
fn child_key(hwnd: HWND, vk: u32, child: HWND) -> bool {
    // SAFETY: one borrow, for two flags and a mode; nothing inside dispatches.
    let focused = unsafe {
        with_sheet(hwnd, |state| {
            if state.name_box_open && child == state.name_box {
                Some(Focused::NameBox)
            } else if state.mode.is_editing() && child == state.editor {
                Some(Focused::Editor(state.mode))
            } else {
                None
            }
        })
    };
    match focused.flatten() {
        Some(Focused::NameBox) => match keymap::key_for(vk) {
            keymap::Key::Return => {
                close_name_box(hwnd, true);
                true
            }
            keymap::Key::Escape => {
                close_name_box(hwnd, false);
                true
            }
            _ => false,
        },
        // The offer list is asked **before** the editing machine, since the keys it claims mean
        // something else there; `assist::on_key` answers `None` unless a list is up.
        Some(Focused::Editor(mode)) => assist_key(hwnd, vk) || on_key(hwnd, mode, vk),
        None => false,
    }
}

/// A keystroke aimed at the completion list, if one is up. `true` means it was claimed.
fn assist_key(hwnd: HWND, vk: u32) -> bool {
    // SAFETY: one borrow, for one bit; nothing inside dispatches.
    let offering = unsafe { with_sheet(hwnd, |state| state.assist.is_offering()) }.unwrap_or(false);
    let Some(reply) = assist::on_key(offering, keymap::key_for(vk), mods()) else {
        return false;
    };
    match reply {
        assist::Reply::Accept => accept_offer(hwnd),
        // Stepping and dismissing change only what the band says, so neither goes near the
        // document, the editor's text or the geometry — the band is already the height it wants.
        assist::Reply::Step(delta) => {
            // SAFETY: one borrow, released before the invalidation.
            unsafe {
                with_sheet(hwnd, |state| {
                    state.assist.step(delta);
                    let pieces = assist::band(&state.assist, state.friendly);
                    state.hint(pieces);
                });
            }
            refresh_bands(hwnd);
        }
        assist::Reply::Dismiss => {
            // SAFETY: one borrow, released before the refresh — which is a full one, since the
            // band going away moves the grid back up under it.
            unsafe {
                with_sheet(hwnd, |state| {
                    state.assist.dismiss();
                    let pieces = assist::band(&state.assist, state.friendly);
                    state.hint(pieces);
                });
            }
            refresh(hwnd);
        }
    }
    true
}

/// Put the highlighted offer into the editor in place of the word being typed — `EM_REPLACESEL`,
/// which leaves the caret after it (for `SUM(`, where the first argument goes) and is one undo
/// step in the control's own history.
fn accept_offer(hwnd: HWND) {
    let Some((edit, text)) = editor_text(hwnd) else {
        return;
    };
    // SAFETY: one borrow, for the span and the replacement; nothing inside dispatches.
    let Some(Some((span, replacement))) =
        (unsafe { with_sheet(hwnd, |state| state.assist.accept()) })
    else {
        return;
    };
    let (from, to) = (
        state::caret_at(&text, span.start),
        state::caret_at(&text, span.end),
    );
    let wide = gdi::wide(&replacement);
    // SAFETY: nothing is borrowed, and the buffer is a NUL-terminated local that outlives the
    // call. Both messages go to this window's own control and dispatch synchronously.
    unsafe {
        select(edit, from, to);
        SendMessageW(
            edit,
            EM_REPLACESEL,
            Some(WPARAM(1)),
            Some(LPARAM(wide.as_ptr() as isize)),
        );
    }
    // The control's `EN_CHANGE` will have arrived already; recompute from the new text so the
    // band shows the signature of the call that was just completed rather than the offers that
    // completed it.
    editor_changed(hwnd);
}

/// Repaint the strip and the two bands under it, and nothing below them.
fn refresh_bands(hwnd: HWND) {
    // SAFETY: one borrow, released before the invalidation.
    let Some(bottom) =
        (unsafe { with_sheet(hwnd, |state| state.geom.header_top().round() as i32) })
    else {
        return;
    };
    // SAFETY: the rectangle is a live local read for the length of the call.
    unsafe {
        let rect = RECT {
            left: 0,
            top: 0,
            right: gdi::client_rect(hwnd).right,
            bottom,
        };
        let _ = InvalidateRect(Some(hwnd), Some(&rect), false);
    }
}

/// What a key means, in whichever mode the grid is in — `sheet/state.rs` decides and this does
/// it. `false` means the key was not claimed: back to `DefWindowProc` from the window, and back
/// to the control from a child, which is what leaves the editor its caret and its own selection.
fn on_key(hwnd: HWND, mode: state::Mode, vk: u32) -> bool {
    let key = keymap::key_for(vk);
    let held = mods();
    // Point mode: where a reference could go, an arrow points at a cell and writes its address
    // (`assist::ref_eligible`); anything else ends it. Only Enter mode starts one — in Edit mode
    // the arrows are the caret's — but a reference already pointed at keeps moving in either.
    if mode.is_editing() {
        let arrow = matches!(
            key,
            keymap::Key::Left | keymap::Key::Right | keymap::Key::Up | keymap::Key::Down
        );
        if arrow && !held.ctrl && !held.shift && !held.alt && point_at(hwnd, mode, key) {
            return true;
        }
        // SAFETY: one borrow, no dialog.
        unsafe { with_sheet(hwnd, |state| state.pointing = None) };
    }
    match state::on_key(mode, key, held) {
        Outcome::Passthrough => false,
        // Opening the name box moves the focus and shows a window, so it happens with nothing
        // borrowed — decision 7's rule, applied to a control rather than to a dialog.
        Outcome::Navigate(keymap::Action::GoTo) => {
            open_name_box(hwnd);
            true
        }
        Outcome::Navigate(action) => {
            // SAFETY: no nested loop inside.
            unsafe {
                with_sheet(hwnd, |state| {
                    state.act(action);
                    sync_scrollbars(hwnd, state);
                });
                let _ = InvalidateRect(Some(hwnd), None, false);
            }
            true
        }
        Outcome::Do(command) => {
            do_command(hwnd, command);
            true
        }
        Outcome::Begin(seed) => {
            begin_edit(hwnd, seed, false);
            true
        }
        Outcome::Commit(dir) => {
            commit_edit(hwnd, dir);
            true
        }
        Outcome::Cancel => {
            cancel_edit(hwnd);
            true
        }
        Outcome::ToggleMode => {
            // SAFETY: no nested loop inside, and nothing on screen changes — F2 changes what the
            // next arrow means.
            unsafe { with_sheet(hwnd, |state| state.mode = state.mode.toggled()) };
            true
        }
    }
}

/// An arrow as point mode: start a reference one cell off the edited cell, or move the one being
/// pointed at, through `EM_REPLACESEL` so Ctrl+Z in the control still undoes it. `false` when a
/// reference could not go here and the arrow is the editor's (or, in Enter mode, a commit).
fn point_at(hwnd: HWND, mode: state::Mode, key: keymap::Key) -> bool {
    let Some((edit, text)) = editor_text(hwnd) else {
        return false;
    };
    let caret = editor_caret(edit, &text);
    // SAFETY: one borrow, nothing inside dispatches.
    let Some((pending, active)) = (unsafe {
        with_sheet(hwnd, |state| {
            (state.pointing.clone(), state.selection.active)
        })
    }) else {
        return false;
    };
    let eligible =
        mode == state::Mode::Enter && grind_sheet::formula::assist::ref_eligible(&text, caret);
    if pending.is_none() && !eligible {
        return false;
    }
    let from = pending.as_ref().map_or(active, |(_, cell)| *cell);
    let cell = match key {
        keymap::Key::Up => Pos::new(from.row.saturating_sub(1), from.col),
        keymap::Key::Down => Pos::new((from.row + 1).min(MAX_ROWS - 1), from.col),
        keymap::Key::Left => Pos::new(from.row, from.col.saturating_sub(1)),
        _ => Pos::new(from.row, (from.col + 1).min(MAX_COLS - 1)),
    };
    let reference = grind_sheet::a1::format(None, cell);
    let span = pending.map_or(caret..caret, |(span, _)| span);
    let wide = gdi::wide(&reference);
    select(
        edit,
        state::caret_at(&text, span.start),
        state::caret_at(&text, span.end),
    );
    // SAFETY: nothing is borrowed, and the buffer is a NUL-terminated local that outlives the
    // call; the message goes to this window's own control and dispatches synchronously.
    unsafe {
        SendMessageW(
            edit,
            EM_REPLACESEL,
            Some(WPARAM(1)),
            Some(LPARAM(wide.as_ptr() as isize)),
        );
    }
    // `editor_changed` ran inside the message above and a keystroke clears nothing: remember the
    // reference after it, so it is the typed text that ends pointing and not the replacement.
    let end = span.start + reference.len();
    // SAFETY: one borrow, no dialog.
    unsafe { with_sheet(hwnd, |state| state.pointing = Some((span.start..end, cell))) };
    true
}

/// A character, after the keyboard layout and the IME; `true` when it started an edit or was
/// otherwise claimed. A character outside the BMP arrives as two `WM_CHAR`s: `surrogate::combine`
/// reassembles them, the first waiting in [`Pane::surrogate_mut`], per pane since a grid and a
/// document are different open documents once `adopt` has run.
fn typed_char(hwnd: HWND, code: u32) -> bool {
    let unit = code as u16;
    // SAFETY: no nested loop inside.
    let resolved = unsafe {
        with_pane(hwnd, |pane| {
            let pending = pane.surrogate_mut();
            if surrogate::is_high(unit) {
                *pending = Some(unit);
                return None;
            }
            match pending.take() {
                Some(high) => surrogate::combine(high, unit),
                // Not a low half completing a pair: either a plain code unit, which
                // `char::from_u32` accepts outright, or a lone low surrogate, which it correctly
                // refuses — there is no pair to make sense of it as.
                None => char::from_u32(code),
            }
        })
    }
    .flatten();
    let Some(c) = resolved else { return false };
    if is_text(hwnd) {
        return text_char(hwnd, c);
    }
    // Space on a checkbox's cell ticks it, rather than starting an edit with a space.
    if c == ' ' {
        // SAFETY: no nested loop inside.
        let ticked = unsafe {
            with_sheet(hwnd, |state| {
                let at = state.selection.active;
                let boxed = state.mode == state::Mode::Ready
                    && state.selection.is_single()
                    && state
                        .app
                        .checkboxes(state.sheet)
                        .is_ok_and(|b| b.iter().any(|(p, _)| *p == at));
                boxed && state.app.toggle_checkbox(state.sheet, at).is_ok()
            })
        };
        if ticked == Some(true) {
            refresh(hwnd);
            return true;
        }
    }
    // SAFETY: no nested loop inside.
    let seed = unsafe { with_sheet(hwnd, |state| state::typed(state.mode, c, mods())) };
    match seed.flatten() {
        Some(seed) => {
            begin_edit(hwnd, seed, false);
            true
        }
        None => false,
    }
}

/// Move a composing IME's composition window to the caret. Only the text pane needs it: the grid
/// edits in a native `EDIT`, which positions its own IME, while this pane draws its own caret, and
/// with `CFS_DEFAULT` the window stays wherever it last was.
fn position_ime_composition(hwnd: HWND) {
    // SAFETY: one borrow, released before the IME calls below; nothing here dispatches.
    let Some((x, y, ..)) = unsafe { with_text(hwnd, |text| text.caret_geometry()) }.flatten()
    else {
        return;
    };
    // SAFETY: `ImmGetContext`/`ImmReleaseContext` are paired within this call, on this thread,
    // with nothing else holding the context in between — `doc/windows-shell.md` decision 7's
    // rule for a nested message loop does not apply here because neither call runs one.
    unsafe {
        let himc = ImmGetContext(hwnd);
        if himc.0.is_null() {
            return;
        }
        let form = COMPOSITIONFORM {
            dwStyle: CFS_POINT,
            ptCurrentPos: POINT { x, y },
            ..Default::default()
        };
        let _ = ImmSetCompositionWindow(himc, &form);
        let _ = ImmReleaseContext(hwnd, himc);
    }
}

/// A press of the left button: what it selects, and what dragging will extend.
fn button_down(hwnd: HWND, lparam: LPARAM) {
    if is_welcome(hwnd) {
        let (x, y) = point(lparam);
        welcome_button_down(hwnd, x, y);
        return;
    }
    if is_text(hwnd) {
        text_button_down(hwnd, lparam);
        return;
    }
    // Clicking elsewhere commits an edit in progress — **before** the selection moves, or the cell
    // being typed into would be the one just clicked.
    commit_edit(hwnd, None);
    let (x, y) = point(lparam);
    // The format strip: **pressed here, done on release** — see `format_button_up`.
    // SAFETY: one borrow; the hit test is arithmetic and dispatches nothing.
    let control = unsafe { with_sheet(hwnd, |state| state.geom.format_hit(x, y)) }.flatten();
    if let Some(control) = control {
        // SAFETY: one borrow; assigning fields dispatches nothing. `SetCapture` and `SetFocus`
        // send messages, which is why they come after it.
        unsafe {
            with_sheet(hwnd, |state| {
                state.format_pressed = Some(control);
                state.format_hover = Some(control);
            });
            let _ = SetCapture(hwnd);
            let _ = SetFocus(Some(hwnd));
            let _ = InvalidateRect(Some(hwnd), None, false);
        }
        return;
    }
    // A sheet tab (`sheet::tabs`): the sheet it names comes to the front, and `+` adds one.
    // SAFETY: one borrow, released before `sheet_add` opens its prompt.
    let tab = unsafe { with_sheet(hwnd, |state| sheet_tab_at(state, x, y)) }.flatten();
    match tab {
        Some(crate::sheet::tabs::Target::Sheet(index)) => {
            sheet_show(hwnd, index);
            return;
        }
        Some(crate::sheet::tabs::Target::Add) => {
            sheet_add(hwnd);
            return;
        }
        // The status bar's spelling language: View ▸ Spelling Language…'s own chooser.
        Some(crate::sheet::tabs::Target::Spelling) => {
            sheet_spelling_language(hwnd);
            return;
        }
        None => {}
    }
    // A chart floats over the cells, so it answers before any of them (`doc/chart-handling.md`): a
    // press on one selects it and takes hold of it — its body to move it, a handle of the selected
    // one to resize it. A press anywhere else lets go of a selected chart.
    // SAFETY: one borrow; arithmetic. `SetCapture` sends messages, so it comes after the borrow.
    let took = unsafe { with_sheet(hwnd, |state| chart_press(state, x, y)) }.flatten();
    if took.is_some() {
        // SAFETY: no borrow held.
        unsafe {
            let _ = SetCapture(hwnd);
            let _ = SetFocus(Some(hwnd));
            let _ = InvalidateRect(Some(hwnd), None, false);
        }
        return;
    }
    // A header boundary: taken to size the track it ends. The size is previewed in `geom` while the
    // pointer moves and written once on release (`button_up`).
    // SAFETY: one borrow; arithmetic. `SetCapture` sends messages, so it comes after the borrow.
    let grabbed = unsafe {
        with_sheet(hwnd, |state| {
            let edge = state.geom.edge_at(x, y)?;
            let (at, size) = match edge {
                Edge::Col(col) => (x, state.geom.cols.size_of(col)),
                Edge::Row(row) => (y, state.geom.rows.size_of(row)),
            };
            state.resize = Some((edge, at, size));
            state.drag = None;
            Some(())
        })
    }
    .flatten();
    if grabbed.is_some() {
        // SAFETY: no borrow held.
        unsafe {
            let _ = SetCapture(hwnd);
            let _ = SetFocus(Some(hwnd));
        }
        return;
    }
    let extend = mods().shift;
    // The two fields on the strip are drawn chrome until they are clicked, at which point the
    // control hiding behind the drawing appears over it. Deciding that needs the geometry, so it is
    // read here and acted on below the borrow.
    // SAFETY: no nested loop inside.
    let click = unsafe {
        with_sheet(hwnd, |state| {
            let hit = state.geom.hit(x, y);
            if hit == Hit::Chrome {
                return Click::Strip(if state.geom.name_box_rect().contains(x, y) {
                    Strip::NameBox
                } else if state.geom.formula_rect().contains(x, y) {
                    Strip::FormulaBar
                } else {
                    Strip::Neither
                });
            }
            // A checkbox's box ticks it — the linked cell's value — and selects its cell.
            if let Hit::Cell { row, col } = hit
                && !extend
                && state
                    .app
                    .checkboxes(state.sheet)
                    .is_ok_and(|b| b.iter().any(|(p, _)| *p == Pos::new(row, col)))
                && state.geom.checkbox(row, col).contains(x, y)
            {
                state.selection = Selection::at(Pos::new(row, col));
                if let Err(error) = state.app.toggle_checkbox(state.sheet, Pos::new(row, col)) {
                    state.say(Some(error.to_string()));
                }
                state.drag = None;
                return Click::Strip(Strip::Neither);
            }
            // The filter's dropdown button lives inside an ordinary cell of its heading row —
            // caught here, before the click becomes a selection, the same way the strip's own
            // two fields are caught above.
            if let Hit::Cell { row, col } = hit
                && let Ok(Some(filter)) = state.app.filter(state.sheet)
                && filter.buttons
                && row == filter.start.row
                && (filter.start.col..=filter.end.col).contains(&col)
                && state
                    .geom
                    .filter_button(row, col)
                    .is_some_and(|b| b.contains(x, y))
            {
                return Click::FilterButton(col);
            }
            state.selection = match (hit, extend) {
                // Shift keeps the anchor and moves the active cell, which is the same rule the
                // keyboard's Shift+arrow follows — one selection model, two ways in.
                (Hit::Cell { row, col }, true) => Selection {
                    anchor: state.selection.anchor,
                    active: Pos::new(row, col),
                },
                (Hit::Cell { row, col }, false) => Selection::at(Pos::new(row, col)),
                (Hit::ColHeader(col), _) => Selection::whole_col(col),
                (Hit::RowHeader(row), _) => Selection::whole_row(row),
                // The corner button selects everything the sheet uses. `Hit::Chrome` is
                // deliberately not this: the status bar is not a select-all button.
                (Hit::Corner, _) => {
                    state.act(keymap::Action::SelectAll);
                    state.selection
                }
                (Hit::Chrome, _) => state.selection,
            };
            state.drag = match hit {
                Hit::Cell { .. } => Some(Drag::Cells),
                Hit::ColHeader(_) => Some(Drag::Cols),
                Hit::RowHeader(_) => Some(Drag::Rows),
                _ => None,
            };
            state.reveal();
            sync_scrollbars(hwnd, state);
            Click::Strip(Strip::Neither)
        })
    };
    match click {
        Some(Click::Strip(Strip::NameBox)) => {
            open_name_box(hwnd);
            return;
        }
        Some(Click::FilterButton(col)) => {
            open_filter_menu(hwnd, col);
            return;
        }
        // Clicking the bar edits the cell it is showing, which is the one place an edit begins
        // somewhere other than on the cell itself.
        Some(Click::Strip(Strip::FormulaBar)) => {
            begin_edit(hwnd, Seed::Cell, true);
            return;
        }
        _ => {}
    }
    // SAFETY: the borrow is released. `SetCapture` and `SetFocus` both send messages
    // synchronously, which is exactly why they are out here.
    unsafe {
        // Capture so that a drag that leaves the window still reports where it went, and take
        // the focus back off the name box if it had it.
        let _ = SetCapture(hwnd);
        let _ = SetFocus(Some(hwnd));
        let _ = InvalidateRect(Some(hwnd), None, false);
    }
}

/// A double-click on a cell opens it for amending — F2, with the mouse, and every spreadsheet's
/// answer. The plain `WM_LBUTTONDOWN` has already selected the cell and started a drag; the drag
/// is cancelled here, because this gesture is not one.
fn double_click(hwnd: HWND, lparam: LPARAM) {
    // A double click on a card is the first click's verb, already run on the release in between —
    // by the time the second arrives the window is showing the document it asked for. Treated as
    // a plain press so that nothing is lost if the pane is somehow still up.
    if is_welcome(hwnd) {
        let (x, y) = point(lparam);
        welcome_button_down(hwnd, x, y);
        return;
    }
    let (x, y) = point(lparam);
    // **A double-click on a format strip is two clicks.** Windows turns the second press of a
    // quick pair into `WM_LBUTTONDBLCLK`, so a strip listening only for the press lost it: `+.0`
    // clicked twice stepped once, and on the text pane the second press selected a word in the
    // document. Treated as a press, and the release activates it.
    if is_text(hwnd) {
        // SAFETY: one borrow; the hit test is arithmetic and dispatches nothing.
        let on_strip = unsafe { with_text(hwnd, |text| text.page.strip_hit(x, y).is_some()) };
        match on_strip {
            Some(true) => text_button_down(hwnd, lparam),
            _ => text_double_click(hwnd, lparam),
        }
        return;
    }
    // SAFETY: one borrow; the hit test is arithmetic and dispatches nothing.
    if unsafe { with_sheet(hwnd, |state| state.geom.format_hit(x, y).is_some()) } == Some(true) {
        button_down(hwnd, lparam);
        return;
    }
    // A double-click on a chart changes it — Return on a selected one, with the mouse. The first
    // click has selected it, so the verb means this one.
    // SAFETY: one borrow; arithmetic.
    let on_chart = unsafe {
        with_sheet(hwnd, |state| {
            let charts = state.app.charts(state.sheet).unwrap_or_default();
            crate::sheet::chart::hit(&charts, &state.geom, state.chart_now(), x, y).is_some()
        })
    };
    if on_chart == Some(true) {
        restyle_chart(hwnd);
        return;
    }
    // A double-click on a header boundary fits the column to its widest text, or gives the row back
    // to its content.
    // SAFETY: one borrow; arithmetic.
    let edge = unsafe { with_sheet(hwnd, |state| state.geom.edge_at(x, y)) }.flatten();
    match edge {
        Some(Edge::Col(col)) => {
            // SAFETY: one borrow; the press that began this pair started a resize.
            unsafe { with_sheet(hwnd, |state| state.resize = None) };
            fit_tracks(hwnd, Fit::Columns(Some(col..col + 1)));
            return;
        }
        Some(Edge::Row(row)) => {
            // SAFETY: one borrow; the write notifies and the observer posts rather than sends.
            unsafe {
                with_sheet(hwnd, |state| {
                    state.resize = None;
                    let _ = state.app.set_row_height(state.sheet, row..row + 1, None);
                })
            };
            refresh(hwnd);
            return;
        }
        None => {}
    }
    // SAFETY: one borrow, released before the rename prompt's nested loop.
    let tab = unsafe { with_sheet(hwnd, |state| sheet_tab_at(state, x, y)) }.flatten();
    if let Some(crate::sheet::tabs::Target::Sheet(_)) = tab {
        sheet_rename(hwnd);
        return;
    }
    // SAFETY: no nested loop inside.
    let on_cell = unsafe {
        with_sheet(hwnd, |state| match state.geom.hit(x, y) {
            Hit::Cell { row, col } => {
                state.selection = Selection::at(Pos::new(row, col));
                state.drag = None;
                true
            }
            _ => false,
        })
    };
    if on_cell == Some(true) {
        begin_edit(hwnd, Seed::Cell, false);
    }
}

fn button_up(hwnd: HWND) {
    if is_welcome(hwnd) {
        welcome_button_up(hwnd);
        return;
    }
    if is_text(hwnd) {
        // A strip control pressed and still under the pointer is activated *now*, after the borrow
        // is released, since some open a modal and no borrow may be held across one (decision 7).
        // SAFETY: one borrow, and nothing inside it dispatches.
        let pressed = unsafe {
            with_text(hwnd, |text| {
                text.dragging = false;
                text.pressed.take().filter(|hit| text.hover == Some(*hit))
            })
        }
        .flatten();
        // SAFETY: the borrow above is released.
        unsafe {
            let _ = ReleaseCapture();
        }
        if let Some(hit) = pressed {
            match hit {
                StripHit::Toggle(which) => text_emphasise(hwnd, STRIP_BUTTONS[which]),
                StripHit::Family => text_pick_family(hwnd),
                StripHit::Size => text_pick_size(hwnd),
                StripHit::Color => text_pick_color(hwnd, false),
                StripHit::Highlight => text_pick_color(hwnd, true),
                StripHit::Clear => text_format(hwnd, grind_text::format::Change::Clear),
            }
        }
        // SAFETY: no borrow held.
        unsafe {
            let _ = InvalidateRect(Some(hwnd), None, false);
        }
        return;
    }
    // A chart let go: a press that barely moved only selected it; any other is one
    // `App::reshape_chart`, one undo step.
    // SAFETY: one borrow; the write notifies and the observer posts rather than sends.
    let released = unsafe {
        with_sheet(hwnd, |state| {
            let grab = state.chart_grab.take()?;
            if grab.is_click(state.geom.dpi) {
                return Some(Ok(()));
            }
            let [x, y, w, h] = crate::sheet::chart::lengths(grab.now, &state.geom);
            Some(
                state
                    .app
                    .reshape_chart(state.sheet, grab.index, &x, &y, &w, &h),
            )
        })
    }
    .flatten();
    if let Some(result) = released {
        // SAFETY: no borrow held.
        unsafe {
            let _ = ReleaseCapture();
        }
        if let Err(error) = result {
            // SAFETY: one borrow, no dialog.
            unsafe { with_sheet(hwnd, |state| state.say(Some(error.to_string()))) };
        }
        refresh(hwnd);
        return;
    }
    // A track let go: its new size is written once, in the document's own unit, as one undo step.
    // SAFETY: one borrow; the write notifies and the observer posts rather than sends.
    let resized = unsafe {
        with_sheet(hwnd, |state| {
            let (edge, _, from) = state.resize.take()?;
            let now = match edge {
                Edge::Col(col) => state.geom.cols.size_of(col),
                Edge::Row(row) => state.geom.rows.size_of(row),
            };
            // A press and release with no drag is a click on the boundary, not a new size.
            if (now - from).abs() < 0.5 {
                return Some(Ok(0));
            }
            let to_mm = 1.0 / (mm_to_px(state.geom.dpi)(1.0) * state.geom.zoom);
            Some(match edge {
                Edge::Col(col) => state.app.set_col_width(
                    state.sheet,
                    col..col + 1,
                    Some(grind_sheet::style::mm_length(
                        state.geom.cols.size_of(col) * to_mm,
                    )),
                ),
                Edge::Row(row) => state.app.set_row_height(
                    state.sheet,
                    row..row + 1,
                    Some(grind_sheet::style::mm_length(
                        state.geom.rows.size_of(row) * to_mm,
                    )),
                ),
            })
        })
    }
    .flatten();
    if let Some(result) = resized {
        // SAFETY: no borrow held.
        unsafe {
            let _ = ReleaseCapture();
        }
        if let Err(error) = result {
            // SAFETY: one borrow, no dialog.
            unsafe { with_sheet(hwnd, |state| state.say(Some(error.to_string()))) };
        }
        refresh(hwnd);
        return;
    }
    // A strip control pressed and still under the pointer is activated now, after the borrow, since
    // two open a modal chooser (decision 7).
    // SAFETY: no nested loop inside.
    let pressed = unsafe {
        with_sheet(hwnd, |state| {
            state.drag = None;
            state
                .format_pressed
                .take()
                .filter(|control| state.format_hover == Some(*control))
        })
    }
    .flatten();
    // SAFETY: the borrow above is released.
    unsafe {
        let _ = ReleaseCapture();
        let _ = InvalidateRect(Some(hwnd), None, false);
    }
    if let Some(control) = pressed {
        format_control(hwnd, control);
    }
}

/// Put the caret in the name box: move the control over the drawn box, fill it, select all, show
/// it — with **nothing borrowed**, since `ShowWindow` and `SetFocus` dispatch messages
/// synchronously.
fn open_name_box(hwnd: HWND) {
    // An edit and an address are two things to type in one window, and only one of them can
    // have the keyboard. Committing rather than cancelling is the same answer a click gets.
    commit_edit(hwnd, None);
    // SAFETY: one borrow, for the three answers, released before anything is shown.
    let Some((edit, rect, text)) = (unsafe {
        with_sheet(hwnd, |state| {
            state.name_box_open = true;
            state.drag = None;
            (
                state.name_box,
                state.geom.name_box_rect(),
                status::name_box_text(&state.app, state.sheet, state.selection),
            )
        })
    }) else {
        return;
    };
    if edit.is_invalid() {
        return;
    }
    let wide = gdi::wide(&text);
    move_to(edit, rect, false);
    // SAFETY: the control is this window's child and outlives the call; the text buffer is a
    // NUL-terminated local, which is what `SetWindowTextW`'s `PCWSTR` wants.
    unsafe {
        let _ = SetWindowTextW(edit, PCWSTR(wide.as_ptr()));
        let _ = ShowWindow(edit, SW_SHOW);
        let _ = SetFocus(Some(edit));
        // Everything selected, so typing an address replaces the one that is there — the
        // behaviour of every other name box, and the reason it is worth pressing F5 twice.
        SendMessageW(edit, EM_SETSEL, Some(WPARAM(0)), Some(LPARAM(-1)));
    }
}

/// Finish with the name box: go where it says, or put it away. Idempotent, and that matters:
/// hiding the control fires `EN_KILLFOCUS`, which calls back in here. The flag is cleared *first*,
/// so the second call returns rather than stealing the focus back.
fn close_name_box(hwnd: HWND, commit: bool) {
    // SAFETY: no nested loop inside.
    let Some((edit, was_open)) = (unsafe {
        with_sheet(hwnd, |state| {
            (
                state.name_box,
                std::mem::replace(&mut state.name_box_open, false),
            )
        })
    }) else {
        return;
    };
    if !was_open || edit.is_invalid() {
        return;
    }
    let typed = if commit {
        window_text(edit)
    } else {
        String::new()
    };
    // SAFETY: the borrow above is released; both calls dispatch messages synchronously.
    unsafe {
        let _ = ShowWindow(edit, SW_HIDE);
        let _ = SetFocus(Some(hwnd));
    }
    // SAFETY: a fresh borrow, taken after the window calls rather than across them.
    unsafe {
        with_sheet(hwnd, |state| {
            // Nonsense goes nowhere: the box closes and the selection stays, less alarming than an
            // error dialog for a typo.
            if let Some(to) = status::locate(&state.app, state.sheet, &typed) {
                state.selection = to;
                state.reveal();
            }
            sync_scrollbars(hwnd, state);
        });
        let _ = InvalidateRect(Some(hwnd), None, false);
    }
}

// ---------------------------------------------------------------------------
// Editing
// ---------------------------------------------------------------------------

/// Start an edit: put the control where the text goes, seed it, and give it the focus.
///
/// `on_bar` forces it onto the formula bar; otherwise [`Sheet::editor_rect`] decides, and picks
/// the bar anyway for a cell that is scrolled out of sight.
fn begin_edit(hwnd: HWND, seed: Seed, on_bar: bool) {
    // SAFETY: one borrow, for the text the seed means; released before [`begin_edit_with`] takes
    // its own, which it must, since it moves a window.
    let Some(text) = (unsafe {
        with_sheet(hwnd, |state| match seed {
            Seed::Char(c) => c.to_string(),
            Seed::Cell => status::formula_bar_text(&state.app, state.sheet, state.selection),
        })
    }) else {
        return;
    };
    begin_edit_with(hwnd, text, seed.mode(), on_bar);
}

/// The same, with the text given rather than derived from a [`Seed`] — the function list's door
/// in, which starts an edit holding `=SUM(` and belongs to no cell's own content.
fn begin_edit_with(hwnd: HWND, text: String, mode: state::Mode, on_bar: bool) {
    // SAFETY: one borrow, for the control and its rectangle.
    let Some((edit, rect)) = (unsafe {
        with_sheet(hwnd, |state| {
            state.mode = mode;
            state.editor_on_bar = on_bar;
            state.drag = None;
            (state.editor, state.editor_rect())
        })
    }) else {
        return;
    };
    if edit.is_invalid() {
        return;
    }
    let wide = gdi::wide(&text);
    let caret = state::caret_at(&text, text.len());
    move_to(edit, rect, false);
    // SAFETY: the borrow is released, which matters because `SetFocus` calls straight back in
    // with the `EN_KILLFOCUS` of whatever had the keyboard before. The text buffer is a
    // NUL-terminated local that outlives the call.
    unsafe {
        let _ = SetWindowTextW(edit, PCWSTR(wide.as_ptr()));
        let _ = ShowWindow(edit, SW_SHOW);
        let _ = SetFocus(Some(edit));
        // The caret goes to the end in both modes: typing over a cell has one character to be
        // after, and F2 opens a cell to be amended rather than replaced.
        select(edit, caret, caret);
        let _ = InvalidateRect(Some(hwnd), None, false);
    }
    // What the seeded text says before a key is pressed: `=SUM(B2:B4)` opened with F2 shows its
    // signature at once, and the function list's `=AVERAGE(` its first argument.
    //
    // **Last**: `SetWindowTextW` fires `EN_CHANGE`, wired to [`editor_changed`], which reads
    // `EM_GETSEL` while the caret is still at zero — a band for an empty prefix, which is none.
    // Asking again here, with the caret placed, makes it stay; `editor_changed` also refreshes,
    // moving this control down when the band took height from the grid.
    editor_changed(hwnd);
}

/// Store what the editor holds and close it, moving the cursor if a key asked to — idempotent for
/// [`close_name_box`]'s reason, since hiding the control fires `EN_KILLFOCUS`.
fn commit_edit(hwnd: HWND, dir: Option<Dir>) {
    // SAFETY: one borrow, for the control and whether there is anything to commit.
    let Some((edit, open)) =
        (unsafe { with_sheet(hwnd, |state| (state.editor, state.mode.is_editing())) })
    else {
        return;
    };
    if !open || edit.is_invalid() {
        return;
    }
    let text = window_text(edit);
    let store = match grind_sheet::formula::display::to_input(&text) {
        Ok(store) => store,
        // A formula that will not parse **does not commit**: the edit stays open with the caret on
        // the problem, rather than storing `=SUM(B2` as text.
        Err(error) => {
            let caret = state::caret_at(&text, error.at);
            // SAFETY: one borrow, for the notice and nothing else.
            unsafe {
                with_sheet(hwnd, |state| {
                    state.say(Some(notice::bad_formula(&error.message)));
                });
            }
            // The banner takes its height out of the grid, so the cell the editor is sitting on
            // has just moved down by one banner. `refresh` is what moves the control with it —
            // and the reason that is a *shared* function rather than a `MoveWindow` here.
            refresh(hwnd);
            // SAFETY: nothing is borrowed; both calls dispatch messages.
            unsafe {
                let _ = SetFocus(Some(edit));
                select(edit, caret, caret);
            }
            return;
        }
    };
    // Closed **first**, so that the `EN_KILLFOCUS` hiding it fires finds nothing left to do.
    // SAFETY: no nested loop inside.
    let Some((sheet, pos)) = (unsafe {
        with_sheet(hwnd, |state| {
            state.editor_on_bar = false;
            state.mode = state::Mode::Ready;
            // Whatever the banner was saying, it was about this edit, and so was the band.
            state.say(None);
            state.assist_done();
            (state.sheet, state.selection.active)
        })
    }) else {
        return;
    };
    // SAFETY: the borrow is released; both calls dispatch messages synchronously.
    unsafe {
        let _ = ShowWindow(edit, SW_HIDE);
        let _ = SetFocus(Some(hwnd));
    }
    // SAFETY: a fresh borrow, taken after the window calls rather than across them. `App::enter`
    // notifies its observer from inside this borrow, which is safe precisely because the
    // observer *posts* rather than sends — see [`Changed`].
    unsafe {
        with_sheet(hwnd, |state| {
            match state.app.enter(sheet, pos, &store, RecalcMode::Document) {
                // A recalculation that was skipped is a state the document is now in, and a
                // state is what the banner is for.
                Ok(outcome) => {
                    if let Some(recalc) = outcome.recalc.filter(|recalc| recalc.spoiled > 0) {
                        state.say(Some(notice::recalc_skipped(recalc.spoiled)));
                    } else if let Some(note) = outcome.note(state.app.locale().as_ref()) {
                        // `1.5` in a German document was read as one and a half — the core's
                        // sentence, as `error.to_string()` is below.
                        state.say(Some(note));
                    }
                }
                Err(error) => state.say(Some(error.to_string())),
            }
            if let Some(dir) = dir {
                state.move_by(dir);
            }
        });
    }
    refresh(hwnd);
}

/// Throw the edit away. The document is not touched, which is the whole promise of Escape.
fn cancel_edit(hwnd: HWND) {
    // SAFETY: no nested loop inside; the flag is cleared here for [`commit_edit`]'s reason.
    let Some((edit, was_open)) = (unsafe {
        with_sheet(hwnd, |state| {
            let was_open = std::mem::replace(&mut state.mode, state::Mode::Ready).is_editing();
            state.editor_on_bar = false;
            state.say(None);
            state.assist_done();
            (state.editor, was_open)
        })
    }) else {
        return;
    };
    if !was_open || edit.is_invalid() {
        return;
    }
    // SAFETY: the borrow is released; both calls dispatch messages synchronously.
    unsafe {
        let _ = ShowWindow(edit, SW_HIDE);
        let _ = SetFocus(Some(hwnd));
    }
    refresh(hwnd);
}

/// The editor's text changed, so the drawn formula bar mirroring it and the assist band are
/// recomputed and repainted — **only those two bands** when the band's height is unchanged, so a
/// keystroke does not redraw the grid; a full [`refresh`] when it appeared or disappeared, which
/// moves the editor too.
fn editor_changed(hwnd: HWND) {
    let (edit, text) = match editor_text(hwnd) {
        Some(pair) => pair,
        None => return,
    };
    let caret = editor_caret(edit, &text);
    // SAFETY: one borrow, released before anything is invalidated. `Sheet::assist` reads
    // `App::names`, which takes the core's read lock and dispatches nothing.
    let Some(moved) = (unsafe { with_sheet(hwnd, |state| state.assist(&text, caret)) }) else {
        return;
    };
    match moved {
        true => refresh(hwnd),
        false => refresh_bands(hwnd),
    }
}

/// The editor control and what it holds, or `None` when no edit is open.
fn editor_text(hwnd: HWND) -> Option<(HWND, String)> {
    // SAFETY: one borrow, for the control and whether it is up; nothing inside dispatches.
    let (edit, open) =
        (unsafe { with_sheet(hwnd, |state| (state.editor, state.mode.is_editing())) })?;
    (open && !edit.is_invalid()).then(|| (edit, window_text(edit)))
}

/// Where the caret is in the editor, as a **byte** offset into `text`: `EM_GETSEL` reports a
/// selection in UTF-16 units, whose end is the caret; [`state::byte_at`] converts.
fn editor_caret(edit: HWND, text: &str) -> usize {
    let mut end: u32 = 0;
    // SAFETY: `edit` is one of this window's controls, and the out-parameter is a live local.
    unsafe {
        SendMessageW(
            edit,
            EM_GETSEL,
            Some(WPARAM(0)),
            Some(LPARAM(std::ptr::from_mut(&mut end) as isize)),
        );
    }
    state::byte_at(text, i32::try_from(end).unwrap_or(i32::MAX))
}

/// `EM_SETSEL`, which counts UTF-16 units — see [`state::caret_at`], which is the conversion.
fn select(edit: HWND, from: i32, to: i32) {
    // SAFETY: `edit` is one of this window's controls and outlives the call.
    unsafe {
        SendMessageW(
            edit,
            EM_SETSEL,
            Some(WPARAM(from as usize)),
            Some(LPARAM(to as isize)),
        );
    }
}

/// A window's text, as a Rust string.
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
// The verbs
// ---------------------------------------------------------------------------

/// Run a verb from a menu item or its accelerator. Every one acts on the **document**, so an edit
/// in progress is committed first — the half-typed cell is part of what Save writes and Undo takes
/// back. Exhaustive on purpose: a command with no handler fails the build, as one in no menu fails
/// `menu.rs`'s test.
fn do_command(hwnd: HWND, command: Command) {
    if is_welcome(hwnd) {
        welcome_command(hwnd, command);
        return;
    }
    if is_text(hwnd) {
        text_command(hwnd, command);
        return;
    }
    // The one verb meaning something *into* an open edit: picking a function while typing inserts
    // the call at the caret, where committing first would try to store a half-formula, which does
    // not commit.
    if matches!(command, Command::FunctionList) {
        function_list(hwnd);
        return;
    }
    commit_edit(hwnd, None);
    match command {
        Command::NewSheet => new_document(hwnd, DocumentKind::Spreadsheet),
        Command::NewText => new_document(hwnd, DocumentKind::Text),
        Command::Welcome => welcome_screen(hwnd),
        Command::Open => open_document(hwnd),
        Command::Save => {
            save(hwnd);
        }
        Command::SaveAs => {
            save_as(hwnd);
        }
        // Not `DestroyWindow`: through `WM_CLOSE`, this item, Alt+F4 and the X are one verb with
        // one close question.
        Command::Exit => {
            // SAFETY: nothing is borrowed, and posting only queues the message.
            unsafe {
                let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
            }
        }
        Command::Undo => history(hwnd, true),
        Command::Redo => history(hwnd, false),
        Command::Cut => copy(hwnd, true),
        Command::Copy => copy(hwnd, false),
        Command::Paste => paste(hwnd),
        Command::ClearCells => clear_cells(hwnd),
        Command::FillDown => fill(hwnd, true),
        Command::FillRight => fill(hwnd, false),
        Command::FillAcross => fill_across(hwnd),
        Command::MergeCells => merge_cells(hwnd, true),
        Command::UnmergeCells => merge_cells(hwnd, false),
        Command::InsertChart => insert_chart(hwnd, None),
        Command::InsertBarChart | Command::InsertLineChart | Command::InsertPieChart => {
            insert_chart(hwnd, command.inserts_chart_kind())
        }
        Command::ChartToBar | Command::ChartToLine | Command::ChartToPie => {
            if let Some(kind) = command.chart_kind() {
                set_chart_kind(hwnd, kind);
            }
        }
        Command::PreviewChart => preview_chart(hwnd),
        Command::DeleteChart => delete_chart(hwnd),
        Command::RestyleChart => restyle_chart(hwnd),
        Command::MoveChart => move_chart(hwnd),
        Command::WrapText => wrap_text(hwnd),
        Command::BordersAll => borders(hwnd, true),
        Command::BordersNone => borders(hwnd, false),
        Command::CopyValue => copy_value(hwnd),
        Command::FormulaToValue => formula_to_value(hwnd),
        Command::HideRows => hide_tracks(hwnd, true, true),
        Command::ShowRows => hide_tracks(hwnd, true, false),
        Command::HideColumns => hide_tracks(hwnd, false, true),
        Command::ShowColumns => hide_tracks(hwnd, false, false),
        Command::RowHeight => track_size(hwnd, true),
        Command::FitColumns => fit_columns(hwnd),
        Command::FitRows => fit_tracks(hwnd, Fit::Rows),
        Command::FitSheet => fit_tracks(hwnd, Fit::Sheet),
        Command::ColumnWidth => track_size(hwnd, false),
        Command::DefineName => define_name(hwnd),
        Command::RenameName => rename_name(hwnd),
        Command::InlineName => inline_name(hwnd),
        Command::DeleteName => delete_name(hwnd),
        Command::DocumentLocale => document_locale(hwnd),
        Command::FindCalculation => find_calculation(hwnd),
        Command::Evaluate => evaluate(hwnd),
        Command::GoTo => open_name_box(hwnd),
        Command::Find => find(hwnd),
        Command::FindNext => find_step(hwnd, find::Towards::Next),
        Command::FindPrevious => find_step(hwnd, find::Towards::Previous),
        Command::Replace => replace(hwnd),
        Command::ImportCsv => import_csv(hwnd, false),
        Command::ImportCsvWith => import_csv(hwnd, true),
        Command::ExportCsv => export_csv(hwnd),
        Command::Recalculate => recalculate(hwnd),
        Command::ToggleFilter => toggle_filter(hwnd),
        Command::FormatTable => format_table(hwnd, None),
        Command::FormatTableTotals => format_table_totals(hwnd),
        Command::AddRule => add_rule(hwnd),
        Command::RemoveRule => remove_rule(hwnd),
        Command::GroupThousands => format_group_thousands(hwnd),
        Command::CurrencyEuro | Command::CurrencyDollar | Command::CurrencyPound => {
            if let Some(index) = command.currency() {
                set_currency(hwnd, index);
            }
        }
        Command::SheetAdd => sheet_add(hwnd),
        Command::SheetRename => sheet_rename(hwnd),
        Command::SheetDelete => sheet_delete(hwnd),
        Command::SheetNext => sheet_step(hwnd, 1),
        Command::SheetPrevious => sheet_step(hwnd, -1),
        Command::ShowSource => show_source(hwnd),
        Command::CheckDocument => check_document(hwnd),
        Command::ToggleRoles => toggle_overlay(hwnd, false),
        Command::ToggleFormulas => toggle_formulas(hwnd),
        Command::ZoomIn => zoom_by(hwnd, 1.25),
        Command::ZoomOut => zoom_by(hwnd, 0.8),
        Command::ZoomReset => zoom_to(hwnd, 1.0),
        Command::ToggleNames => toggle_overlay(hwnd, true),
        // Handled above, before the commit — but the match stays exhaustive, which is what says
        // every command has a handler.
        Command::FunctionList => function_list(hwnd),
        Command::ExplainFormula => explain_formula(hwnd),
        Command::ToggleFriendly => toggle_friendly(hwnd),
        // The format strip's verbs, from the menu or a key — the same `format_control` a click on
        // the strip reaches, so the two cannot mean different things.
        Command::Bold => format_control(hwnd, format::Control::Bold),
        Command::Italic => format_control(hwnd, format::Control::Italic),
        Command::Underline => format_control(hwnd, format::Control::Underline),
        Command::Strike => format_control(hwnd, format::Control::Strike),
        Command::AlignLeft => format_control(hwnd, format::Control::AlignLeft),
        Command::AlignCenter => format_control(hwnd, format::Control::AlignCenter),
        Command::AlignRight => format_control(hwnd, format::Control::AlignRight),
        Command::IndentLess => format_control(hwnd, format::Control::Outdent),
        Command::IndentMore => format_control(hwnd, format::Control::Indent),
        Command::TextRotation => format_pick_rotation(hwnd),
        Command::PickColor => format_control(hwnd, format::Control::Color),
        Command::PickBackground => format_control(hwnd, format::Control::Background),
        Command::NumberFormat => format_control(hwnd, format::Control::Number),
        Command::FewerDecimals => format_control(hwnd, format::Control::FewerDecimals),
        Command::MoreDecimals => format_control(hwnd, format::Control::MoreDecimals),
        Command::ClearFormatting => format_control(hwnd, format::Control::Clear),
        // The text pane's, and this one has no run, block or outline to work with.
        Command::Code
        | Command::PickFamily
        | Command::PickSize
        | Command::PickHighlight
        | Command::Title
        | Command::Subtitle
        | Command::Paragraph
        | Command::Heading1
        | Command::Heading2
        | Command::Heading3
        | Command::Outline
        | Command::BlockKindDialog
        | Command::InsertPicture
        | Command::AltText
        | Command::InsertTable
        | Command::Bookmark
        | Command::ParagraphStyle
        | Command::ParagraphUp
        | Command::ParagraphDown
        | Command::ParagraphDelete
        | Command::ImportMarkdown
        | Command::ExportMarkdown
        | Command::ExportPdf
        | Command::PrintPreview
        | Command::Print => {}
        Command::NextMisspelling => sheet_next_misspelling(hwnd),
        Command::SpellingLanguage => sheet_spelling_language(hwnd),
        Command::PersonalDictionary => personal_dictionary(hwnd),
        Command::ToggleSpelling => sheet_toggle_spelling(hwnd),
        Command::Shortcuts => show_shortcuts(hwnd),
        Command::About => dialog::about(hwnd),
        Command::Licences => dialog::licences(hwnd),
    }
}

/// Undo or redo. The history is the core's — architecture rule 2 — and this is the whole of what
/// a shell does about it.
fn history(hwnd: HWND, undo: bool) {
    // SAFETY: one borrow. `App::undo` notifies, and the observer posts rather than sends.
    unsafe {
        with_sheet(hwnd, |state| {
            let did = match undo {
                true => state.app.undo(),
                false => state.app.redo(),
            };
            if did {
                // Whatever the banner said was about the document as it was.
                state.say(None);
            }
        });
    }
    refresh(hwnd);
}

/// Put the selection on the clipboard as `CF_UNICODETEXT`, tab- and CRLF-separated
/// (`grind_sheet::clip::rect_text`), and with `cut` clear it — one `App::clear_range`, one undo
/// step. Each cell's `App::input_text` travels, not what it displays (decision 6): pasted back it
/// reproduces the cells, and into Calc or Excel `1234.5` is a number where `1,234.50 €` is a
/// locale guess.
fn copy(hwnd: HWND, cut: bool) {
    // SAFETY: one borrow; `clipboard::set_text` and `clear_range` each run with nothing else
    // borrowed.
    unsafe {
        with_sheet(hwnd, |state| {
            // A whole row or column is cut to what the sheet uses, or a copy of a column would
            // be a million lines (`nav::target`).
            let used = state.app.used_extent(state.sheet).unwrap_or((0, 0));
            let (start, end) = grind_sheet::nav::target(state.selection, used);
            let text =
                clip::rect_text(&state.app, state.sheet, start, end, App::input_text, "\r\n");
            clipboard::set_text(hwnd, &text);
            if cut && let Err(error) = state.app.clear_range(state.sheet, start, end) {
                state.say(Some(error.to_string()));
            }
        });
    }
    refresh(hwnd);
}

/// Read the clipboard and fill from the selection's top-left — `App::enter_range` under
/// `grind_sheet::clip::parse_rows`, one undo step. No text on the clipboard (an image, a file
/// list) does nothing.
fn paste(hwnd: HWND) {
    let Some(text) = clipboard::get_text(hwnd) else {
        return;
    };
    let rows = clip::parse_rows(&text);
    // SAFETY: one borrow. `enter_range` notifies, and the observer posts rather than sends.
    unsafe {
        with_sheet(hwnd, |state| {
            let (start, _) = state.selection.rect();
            match state
                .app
                .enter_range(state.sheet, start, &rows, RecalcMode::Document)
            {
                Ok(outcome) => {
                    if let Some(recalc) = outcome.recalc.filter(|r| r.spoiled > 0) {
                        state.say(Some(notice::recalc_skipped(recalc.spoiled)));
                    } else if let Some(note) = outcome.note(state.app.locale().as_ref()) {
                        state.say(Some(note));
                    }
                    let last = Pos::new(
                        start.row + rows.len().saturating_sub(1) as u32,
                        start.col
                            + rows
                                .iter()
                                .map(Vec::len)
                                .max()
                                .unwrap_or(1)
                                .saturating_sub(1) as u32,
                    );
                    state.selection = Selection {
                        anchor: start,
                        active: last,
                    };
                    state.reveal();
                    sync_scrollbars(hwnd, state);
                }
                Err(error) => state.say(Some(error.to_string())),
            }
        });
    }
    refresh(hwnd);
}

/// Empty the selected cells, keeping their formatting — Delete's verb, and the Edit menu's.
fn clear_cells(hwnd: HWND) {
    // SAFETY: one borrow; `clear_range` is one `Action::Batch`, so this is one Ctrl+Z.
    unsafe {
        with_sheet(hwnd, |state| {
            let (start, end) = state.selection.rect();
            if let Err(error) = state.app.clear_range(state.sheet, start, end) {
                state.say(Some(error.to_string()));
            }
        });
    }
    refresh(hwnd);
}

/// Edit ▸ Fill Down / Fill Right: one `App::fill` per column (or row), each from its own leading
/// cell, over the lines `grind_sheet::nav::fills` names — so a two-column fill copies column A
/// into A and column B into B.
fn fill(hwnd: HWND, down: bool) {
    // SAFETY: one borrow, no dialog.
    unsafe {
        with_sheet(hwnd, |state| {
            let mut cells = 0;
            for (source, start, end) in grind_sheet::nav::fills(state.selection, down) {
                match state
                    .app
                    .fill(state.sheet, source, start, end, RecalcMode::Document)
                {
                    Ok(done) => cells += done.cells as u64,
                    Err(error) => return state.say(Some(error.to_string())),
                }
            }
            state.say(Some(notice::filled(cells, down)));
        });
    }
    refresh(hwnd);
}

/// Sheet ▸ Merge Cells and Unmerge Cells — `App::merge`/`App::unmerge` over the selection, a whole
/// row or column cut to the part in use (`nav::target`); what the covered cells held stays.
fn merge_cells(hwnd: HWND, merge: bool) {
    // SAFETY: one borrow, no dialog.
    unsafe {
        with_sheet(hwnd, |state| {
            let used = state.app.used_extent(state.sheet).unwrap_or((0, 0));
            let (start, end) = grind_sheet::nav::target(state.selection, used);
            let said = match merge {
                true => match state.app.merge(state.sheet, start, end) {
                    Ok(_) => {
                        state.selection = Selection::at(start);
                        None
                    }
                    Err(error) => Some(error.to_string()),
                },
                false => match state.app.unmerge(state.sheet, start, end) {
                    Ok(_) => None,
                    Err(error) => Some(error.to_string()),
                },
            };
            state.say(said);
        });
    }
    refresh(hwnd);
}

/// Chart ▸ Insert Chart, and Insert Bar/Line/Pie Chart — `grind_sheet::verbs::insert_chart` over
/// the selection, as `kind` or as the kind the cells want; this window says only where a column
/// and a row sit, in millimetres.
fn insert_chart(hwnd: HWND, kind: Option<grind_sheet::ChartKind>) {
    // SAFETY: one borrow, no dialog.
    unsafe {
        with_sheet(hwnd, |state| {
            let (start, end) = state.selection.rect();
            let px = 1.0 / (crate::sheet::geom::mm_to_px(state.geom.dpi)(1.0) * state.geom.zoom);
            let result = grind_sheet::verbs::insert_chart(
                &state.app,
                state.sheet,
                start,
                end,
                kind,
                |col, row| {
                    (
                        state.geom.cols.offset_of(col) * px,
                        state.geom.rows.offset_of(row) * px,
                    )
                },
            );
            state.say(Some(match result {
                Ok(_) => "A chart beside the table. Ctrl+Z takes it back.".to_owned(),
                Err(error) => error.to_string(),
            }));
        });
    }
    refresh(hwnd);
}

/// Chart ▸ Chart Preview… — the chart Insert would make, in a dialog with the three kinds to
/// press, opening on the one the cells want; **Insert** is the menu's `insert_chart` as the kind
/// pressed, Cancel writes nothing. The borrow is released first (decision 7).
fn preview_chart(hwnd: HWND) {
    // SAFETY: one borrow, ended before the modal below.
    let made = unsafe {
        with_sheet(hwnd, |state| {
            let (start, end) = state.selection.rect();
            let preview = |kind| {
                grind_sheet::verbs::preview_insert_chart(&state.app, state.sheet, start, end, kind)
            };
            let made = preview(None).and_then(|(guessed, _)| {
                let pictures = grind_sheet::ChartKind::ALL
                    .into_iter()
                    .map(|kind| preview(Some(kind)).map(|(chart, data)| (kind, chart, data)))
                    .collect::<grind_sheet::Result<Vec<_>>>()?;
                let shown = grind_sheet::ChartKind::ALL
                    .iter()
                    .position(|kind| *kind == guessed.kind)
                    .unwrap_or(0);
                Ok((pictures, shown))
            });
            if let Err(error) = &made {
                state.say(Some(error.to_string()));
            }
            made.ok()
                .map(|(pictures, shown)| (pictures, shown, state.geom.dpi))
        })
        .flatten()
    };
    if let Some((pictures, shown, dpi)) = made
        && let Some(kind) = crate::dialog::chart_preview(hwnd, pictures, shown, dpi)
    {
        insert_chart(hwnd, Some(kind));
        return;
    }
    refresh(hwnd);
}

/// Chart ▸ Change to Bar/Line/Pie Chart, and the same three on a chart's own menu — the selected
/// chart, else the last, turned into `kind` (`verbs::set_chart_kind`).
fn set_chart_kind(hwnd: HWND, kind: grind_sheet::ChartKind) {
    // SAFETY: one borrow, no dialog.
    unsafe {
        with_sheet(hwnd, |state| {
            state.say(Some(match state.chart_target() {
                None => "This sheet has no chart.".to_owned(),
                Some(index) => {
                    match grind_sheet::verbs::set_chart_kind(&state.app, state.sheet, index, kind) {
                        Ok(said) => format!("{said} Ctrl+Z takes it back."),
                        Err(why) => why,
                    }
                }
            }));
        });
    }
    refresh(hwnd);
}

/// Chart ▸ Change Chart… — the selected chart, else the last: its kind, title and legend in words
/// (`verbs::restyle_chart`). Return on a selected chart, a double click on one, and its menu.
fn restyle_chart(hwnd: HWND) {
    let Some(words) = dialog::prompt(
        hwnd,
        "Change Chart",
        &format!("{}:", grind_sheet::verbs::CHART_WORDS),
        "",
    ) else {
        return;
    };
    // SAFETY: one borrow, after the dialog.
    unsafe {
        with_sheet(hwnd, |state| {
            state.say(Some(match state.chart_target() {
                None => "This sheet has no chart.".to_owned(),
                Some(index) => {
                    match grind_sheet::verbs::restyle_chart(&state.app, state.sheet, index, &words)
                    {
                        Ok(said) => format!("{said} Ctrl+Z takes it back."),
                        Err(why) => why,
                    }
                }
            }));
        });
    }
    refresh(hwnd);
}

/// Chart ▸ Move Chart Here — the selected chart, else the last: its corner to the active cell's,
/// its size kept (`verbs::move_chart`), over the offsets this window draws the grid at.
fn move_chart(hwnd: HWND) {
    // SAFETY: one borrow, no dialog.
    unsafe {
        with_sheet(hwnd, |state| {
            let px = 1.0 / (crate::sheet::geom::mm_to_px(state.geom.dpi)(1.0) * state.geom.zoom);
            state.say(Some(match state.chart_target() {
                None => "This sheet has no chart.".to_owned(),
                Some(index) => match grind_sheet::verbs::move_chart(
                    &state.app,
                    state.sheet,
                    index,
                    state.selection.active,
                    |col, row| {
                        (
                            state.geom.cols.offset_of(col) * px,
                            state.geom.rows.offset_of(row) * px,
                        )
                    },
                ) {
                    Ok(said) => format!("{said} Ctrl+Z takes it back."),
                    Err(why) => why,
                },
            }));
        });
    }
    refresh(hwnd);
}

/// Chart ▸ Delete Chart — the selected chart (Delete, Backspace, its menu), else the last.
fn delete_chart(hwnd: HWND) {
    // SAFETY: one borrow, no dialog.
    unsafe {
        with_sheet(hwnd, |state| {
            let target = state.chart_target();
            state.chart_selected = None;
            state.say(Some(match target {
                None => "This sheet has no chart.".to_owned(),
                Some(index) => match state.app.remove_chart(state.sheet, index) {
                    Ok(()) => grind_sheet::chart_frame::deleted_sentence("Ctrl+Z"),
                    Err(error) => error.to_string(),
                },
            }));
        });
    }
    refresh(hwnd);
}

/// Edit ▸ Fill Across — the cell the selection grew from, into the whole selection, references shifted, as one
/// `App::fill` and so one undo step.
fn fill_across(hwnd: HWND) {
    // SAFETY: one borrow, no dialog.
    unsafe {
        with_sheet(hwnd, |state| {
            let (start, end) = state.selection.rect();
            state.say(Some(
                match state.app.fill(
                    state.sheet,
                    // The cell the selection grew from, which is the one somebody means.
                    state.selection.anchor,
                    start,
                    end,
                    RecalcMode::Document,
                ) {
                    Ok(done) => notice::filled(done.cells as u64, true),
                    Err(error) => error.to_string(),
                },
            ));
        });
    }
    refresh(hwnd);
}

/// Format ▸ Wrap Text — `Toggle::Wrap` flipped over the selection, from the active cell's state.
fn wrap_text(hwnd: HWND) {
    format_write(hwnd, |state, start, end| {
        let style = active_style(state);
        state.app.set_style(
            state.sheet,
            start,
            end,
            grind_sheet::format::Toggle::Wrap.flipped(&style),
        )
    });
}

/// Format ▸ All Borders / No Borders — read the active cell's style, set or clear the hairline on
/// every edge, write it over the selection: `grind_sheet::format::bordered`, one undo step.
fn borders(hwnd: HWND, on: bool) {
    // SAFETY: one borrow, no dialog.
    unsafe {
        with_sheet(hwnd, |state| {
            let (start, end) = state.selection.rect();
            let style = state
                .app
                .style_at(state.sheet, state.selection.active)
                .ok()
                .flatten()
                .unwrap_or_default();
            if let Err(error) = state.app.set_style(
                state.sheet,
                start,
                end,
                grind_sheet::format::bordered(&style, on),
            ) {
                state.say(Some(error.to_string()));
            }
        });
    }
    refresh(hwnd);
}

/// Edit ▸ Copy Value — the selection as it is *shown*, on the clipboard as tab-separated text
/// (`App::value_text`). The bytes of Copy's own rectangle, with the answer instead of the source.
fn copy_value(hwnd: HWND) {
    // SAFETY: one borrow; `set_text` runs with nothing else borrowed.
    unsafe {
        with_sheet(hwnd, |state| {
            let used = state.app.used_extent(state.sheet).unwrap_or((0, 0));
            let (start, end) = grind_sheet::nav::target(state.selection, used);
            let text =
                clip::rect_text(&state.app, state.sheet, start, end, App::value_text, "\r\n");
            clipboard::set_text(hwnd, &text);
        });
    }
}

/// Edit ▸ Formula to Value — every formula in the selection dropped, each cell keeping the value
/// it last computed.
fn formula_to_value(hwnd: HWND) {
    // SAFETY: one borrow, no dialog.
    unsafe {
        with_sheet(hwnd, |state| {
            let used = state.app.used_extent(state.sheet).unwrap_or((0, 0));
            let (start, end) = grind_sheet::nav::target(state.selection, used);
            state.say(Some(
                match grind_sheet::verbs::formulas_to_values(&state.app, state.sheet, start, end) {
                    Ok(n) => notice::formulas_dropped(n),
                    Err(error) => error.to_string(),
                },
            ));
        });
    }
    refresh(hwnd);
}

/// Sheet ▸ Hide / Show Rows or Columns — the run the selection spans, `App::set_row_hidden` /
/// `set_col_hidden`, one undo step. Showing reaches a hidden run because a selection made with
/// Shift+arrow across one still spans its indexes.
fn hide_tracks(hwnd: HWND, rows: bool, hidden: bool) {
    // SAFETY: one borrow, no dialog.
    unsafe {
        with_sheet(hwnd, |state| {
            let result = match rows {
                true => state.app.set_row_hidden(
                    state.sheet,
                    grind_sheet::verbs::rows(state.selection),
                    hidden,
                ),
                false => state.app.set_col_hidden(
                    state.sheet,
                    grind_sheet::verbs::cols(state.selection),
                    hidden,
                ),
            };
            state.say(Some(match result {
                Ok(count) => notice::tracks_hidden(count, rows, hidden),
                Err(error) => error.to_string(),
            }));
        });
    }
    refresh(hwnd);
}

/// Sheet ▸ Row Height… / Column Width… — a length in a prompt (`2.5cm`, `64pt`), over every
/// selected row or column. `App` checks the length, so a nonsense one is the notice's problem,
/// not a column that silently vanishes. An empty answer clears the size back to the default.
fn track_size(hwnd: HWND, rows: bool) {
    let prompt = match rows {
        true => "Row height — 2.5cm, 1in or 64pt; empty for the default:",
        false => "Column width — 2.5cm, 1in or 64pt; empty for the default:",
    };
    let title = match rows {
        true => "Row Height",
        false => "Column Width",
    };
    let Some(answer) = dialog::prompt(hwnd, title, prompt, "") else {
        return;
    };
    let size = Some(answer.trim().to_owned()).filter(|size| !size.is_empty());
    // SAFETY: one borrow, taken after the dialog.
    unsafe {
        with_sheet(hwnd, |state| {
            let result = match rows {
                true => state.app.set_row_height(
                    state.sheet,
                    grind_sheet::verbs::rows(state.selection),
                    size,
                ),
                false => state.app.set_col_width(
                    state.sheet,
                    grind_sheet::verbs::cols(state.selection),
                    size,
                ),
            };
            state.say(Some(match result {
                Ok(count) => notice::track_sized(count, rows),
                Err(error) => error.to_string(),
            }));
        });
    }
    refresh(hwnd);
}

/// Sheet ▸ Fit Column Width — each selected column as wide as its widest text, measured in its
/// drawing font (decision 3), with padding; an empty column goes back to the default.
fn fit_columns(hwnd: HWND) {
    fit_tracks(hwnd, Fit::Columns(None));
}

/// What a fit covers: some columns (the selected ones when none are named — a double-click on a
/// column's edge names its own), the selected rows, or the whole sheet.
enum Fit {
    Columns(Option<std::ops::Range<u32>>),
    Rows,
    Sheet,
}

/// Columns fitted to their widest text and rows given back to their content, as one undo step
/// (`App::fit`): Sheet ▸ Fit Column Width, Fit Row Height and Fit Content to Cells.
fn fit_tracks(hwnd: HWND, fit: Fit) {
    // SAFETY: one borrow, no dialog.
    unsafe {
        with_sheet(hwnd, |state| {
            let sheet = state.sheet;
            let (start, end) = state.selection.rect();
            let rows_only = matches!(fit, Fit::Rows);
            let (cols, rows) = match fit {
                Fit::Columns(cols) => (
                    grind_sheet::fit::columns_in_use(
                        &state.app,
                        sheet,
                        cols.unwrap_or(start.col..end.col + 1),
                    ),
                    0..0,
                ),
                Fit::Rows => (0..0, start.row..end.row + 1),
                Fit::Sheet => (
                    grind_sheet::fit::all_columns(&state.app, sheet),
                    0..u32::MAX,
                ),
            };
            let Some(dib) = gdi::Dib::new(1, 1) else {
                return;
            };
            let dc = dib.dc();
            let dpi = state.geom.dpi;
            let px = scale(theme::text::CELL, dpi).round() as i32;
            let (regular, bold) = (
                gdi::Font::new(face(), px, false),
                gdi::Font::new(face(), px, true),
            );
            let used = state.app.used_extent(sheet).unwrap_or((0, 0)).0.min(5000);
            let pad = 2.0 * scale(4.0, dpi) + 6.0;
            let to_mm = 1.0 / crate::sheet::geom::mm_to_px(dpi)(1.0);
            let widths = cols
                .map(|col| {
                    let widest = state
                        .app
                        .get_viewport(sheet, 0..used, col..col + 1)
                        .map(|view| {
                            (0..used)
                                .filter_map(|row| {
                                    let text =
                                        view.text(row, col).filter(|text| !text.is_empty())?;
                                    let heavy = view.style(row, col).is_some_and(|style| {
                                        grind_sheet::format::Toggle::Bold.is_on(style)
                                    });
                                    let _font = gdi::Selected::font(
                                        dc,
                                        if heavy { &bold } else { &regular },
                                    );
                                    Some(f64::from(gdi::text_width(dc, text)))
                                })
                                .fold(0.0, f64::max)
                        })
                        .unwrap_or(0.0);
                    (col, grind_sheet::fit::width(widest, pad, to_mm))
                })
                .collect();
            let changed = state.app.fit(sheet, widths, rows).unwrap_or(0);
            state.say(Some(notice::track_sized(changed, rows_only)));
        });
    }
    refresh(hwnd);
}

/// Data ▸ Find a Calculation… — a word (empty for all), then every formula whose sheet, address,
/// formula text or function name has it, in a list; choosing one goes to its cell.
fn find_calculation(hwnd: HWND) {
    let Some(word) = dialog::prompt(
        hwnd,
        "Find a Calculation",
        "A word in the formula, its address or a function it calls (empty lists all):",
        "",
    ) else {
        return;
    };
    // SAFETY: one borrow, released before the list — which runs a nested message loop.
    let Some(found) = (unsafe {
        with_sheet(hwnd, |state| {
            state
                .app
                .calculations()
                .into_iter()
                .filter(|calc| calc.matches(word.trim()))
                .collect::<Vec<_>>()
        })
    }) else {
        return;
    };
    if found.is_empty() {
        // SAFETY: one borrow, no dialog.
        unsafe {
            with_sheet(hwnd, |state| {
                state.say(Some(
                    "Nothing here is calculated, or nothing matches. A cell starting with = is."
                        .to_owned(),
                ))
            })
        };
        return refresh(hwnd);
    }
    let rows: Vec<String> = found
        .iter()
        .map(|calc| format!("{}   {} = {}", calc.address(), calc.formula, calc.value))
        .collect();
    let Some(at) = dialog::choose(hwnd, "Calculations", &rows, 0) else {
        return;
    };
    if let Some(calc) = found.get(at) {
        go_to_address(hwnd, &calc.address());
    }
}

/// Data ▸ Document Locale… — a tag (`de-DE`, `fr`; empty for none) for how this document spells and
/// reads numbers. `grind_sheet::verbs::locale` reads the tag, `App::set_locale` is one undo step.
fn document_locale(hwnd: HWND) {
    // SAFETY: one borrow, released before the prompt.
    let Some(now) = (unsafe {
        with_sheet(hwnd, |state| {
            state.app.locale().map(|l| l.tag()).unwrap_or_default()
        })
    }) else {
        return;
    };
    let Some(typed) = dialog::prompt(
        hwnd,
        "Document Locale",
        "A locale tag such as de-DE (empty for none):",
        &now,
    ) else {
        return;
    };
    // SAFETY: one borrow, after the dialog.
    unsafe {
        with_sheet(hwnd, |state| {
            state.say(Some(match grind_sheet::verbs::locale(&typed) {
                Ok(locale) => match state.app.set_locale(locale) {
                    Ok(()) => "The document's locale is set. Ctrl+Z takes it back.".to_owned(),
                    Err(error) => error.to_string(),
                },
                Err(_) => format!("“{}” is not a locale — try de-DE.", typed.trim()),
            }));
        });
    }
    refresh(hwnd);
}

/// Data ▸ Define Name… — a name for the selection, sheet-qualified so it means the same place from
/// every sheet (`grind_sheet::verbs::name_target`). Redefining one is `grind sheet name`'s.
fn define_name(hwnd: HWND) {
    // SAFETY: one borrow, released before the prompt.
    let Some(target) = (unsafe {
        with_sheet(hwnd, |state| {
            let sheet = state.app.sheet_name(state.sheet).unwrap_or_default();
            grind_sheet::verbs::name_target(&sheet, state.selection)
        })
    }) else {
        return;
    };
    let Some(name) = dialog::prompt(
        hwnd,
        "Define Name",
        &format!("A name for {target}, to use in formulas instead of its address:"),
        "",
    ) else {
        return;
    };
    let name = name.trim().to_owned();
    if name.is_empty() {
        return;
    }
    // SAFETY: a fresh borrow, after the dialog.
    unsafe {
        with_sheet(hwnd, |state| {
            let result = grind_sheet::a1::definition(&state.app, &target)
                .and_then(|expression| state.app.set_name(&name, &expression));
            state.say(Some(match result {
                Ok(()) => notice::name_defined(&name, &target),
                Err(error) => error.to_string(),
            }));
        });
    }
    refresh(hwnd);
}

/// The name asked for in a prompt, trimmed, or `None` when cancelled or empty.
fn ask_name(hwnd: HWND, title: &str, prompt: &str, default: &str) -> Option<String> {
    let name = dialog::prompt(hwnd, title, prompt, default)?;
    let name = name.trim().to_owned();
    (!name.is_empty()).then_some(name)
}

/// Names ▸ Rename… — the name and its new spelling; every formula and every other name that uses
/// it follows (`App::rename_name`, one undo step).
fn rename_name(hwnd: HWND) {
    let Some(from) = ask_name(hwnd, "Rename Name", "Rename which name:", "") else {
        return;
    };
    let Some(to) = ask_name(hwnd, "Rename Name", &format!("Rename “{from}” to:"), &from) else {
        return;
    };
    // SAFETY: one borrow, after the dialogs.
    unsafe {
        with_sheet(hwnd, |state| {
            state.say(Some(match state.app.rename_name(&from, &to) {
                Ok(uses) => format!(
                    "“{from}” is “{to}” now — {uses} use(s) rewritten. Ctrl+Z takes it back."
                ),
                Err(error) => error.to_string(),
            }));
        });
    }
    refresh(hwnd);
}

/// Names ▸ Inline into Formulas… — the name's definition written into every use, and the name
/// dropped (`App::inline_name`, one undo step).
fn inline_name(hwnd: HWND) {
    let Some(name) = ask_name(hwnd, "Inline Name", "Inline which name:", "") else {
        return;
    };
    // SAFETY: one borrow, after the dialog.
    unsafe {
        with_sheet(hwnd, |state| {
            state.say(Some(match state.app.inline_name(&name) {
                Ok(uses) => format!("“{name}” inlined — {uses} use(s) rewritten, the name is gone. Ctrl+Z takes it back."),
                Err(error) => error.to_string(),
            }));
        });
    }
    refresh(hwnd);
}

/// Names ▸ Delete… — the definition only; a formula that used it will say `#NAME?`.
fn delete_name(hwnd: HWND) {
    let Some(name) = ask_name(hwnd, "Delete Name", "Delete which name:", "") else {
        return;
    };
    // SAFETY: one borrow, after the dialog.
    unsafe {
        with_sheet(hwnd, |state| {
            state.say(Some(match state.app.clear_name(&name) {
                true => format!("“{name}” deleted. Ctrl+Z takes it back."),
                false => format!("There is no name “{name}”."),
            }));
        });
    }
    refresh(hwnd);
}

/// Data ▸ Evaluate… — a formula typed in display syntax, worked out at the active cell and said in
/// the notice bar. `App::preview`: nothing stored, no undo step.
fn evaluate(hwnd: HWND) {
    let Some(typed) = dialog::prompt(
        hwnd,
        "Evaluate",
        "A formula, worked out at the active cell without storing it:",
        "=",
    ) else {
        return;
    };
    // SAFETY: one borrow, after the dialog.
    unsafe {
        with_sheet(hwnd, |state| {
            let at = state.selection.active;
            state.say(Some(
                match grind_sheet::verbs::evaluated(&state.app, state.sheet, at, &typed) {
                    Ok(value) => notice::evaluated(typed.trim(), &value),
                    Err(why) => format!("That cannot be worked out: {why}"),
                },
            ));
        });
    }
    refresh(hwnd);
}

// --- the source, the check, and the overlays ---

/// The document as its projection, opened on the line the selection or caret projects to
/// (`Projection::line_of`) — read-only, a modal list (`code.rs`) rather than a drawn pane.
fn show_source(hwnd: HWND) {
    // SAFETY: one borrow, released before the modal — which runs a nested message loop.
    let Some(projection) = (unsafe { with_pane(hwnd, project) }).flatten() else {
        return;
    };
    let rows = code::rows(&projection);
    if rows.is_empty() {
        dialog::error(hwnd, "This document has no source to show.");
        return;
    }
    let initial = current_address(hwnd)
        .and_then(|address| projection.line_of(&address))
        .unwrap_or(0);
    let Some(line) = dialog::choose(hwnd, "Show Source", &rows, initial) else {
        return;
    };
    if let Some(target) = projection.address_on_line(line) {
        go_to_address(hwnd, target);
    }
}

/// Check Document — every finding `App::lint` reports, worst first, each row a jump.
fn check_document(hwnd: HWND) {
    // SAFETY: one borrow, released before the modal.
    let Some(report) = (unsafe { with_pane(hwnd, lint) }).flatten() else {
        return;
    };
    if report.is_empty() {
        dialog::error(hwnd, "No problems found.");
        return;
    }
    let rows = problems::rows(&report);
    let Some(choice) = dialog::choose(hwnd, "Check Document", &rows, 0) else {
        return;
    };
    let Some(diagnostic) = report.diagnostics.get(choice) else {
        return;
    };
    // Empty for a finding about the document as a whole, which is nowhere to jump to.
    if !diagnostic.at.is_empty() {
        go_to_address(hwnd, &diagnostic.at.clone());
    }
}

/// `doc/view-modes.md`'s two overlays, flipped. Menu-only — neither has a key of its own, the
/// same as the four sheet verbs beside them — and the menu is rebuilt straight after so its
/// checkmark answers with the state it just changed to rather than the one before.
fn toggle_overlay(hwnd: HWND, names: bool) {
    // SAFETY: one borrow; nothing inside dispatches.
    unsafe {
        with_sheet(hwnd, |state| match names {
            true => state.overlays.names = !state.overlays.names,
            false => state.overlays.roles = !state.overlays.roles,
        });
    }
    build_menu(hwnd);
    refresh(hwnd);
}

/// View ▸ Zoom In / Out / 100% — a factor on every cell (25%–400%), a reading that is never stored:
/// nothing measured is saved zoomed. Ctrl+wheel does the same.
fn zoom_by(hwnd: HWND, factor: f64) {
    // SAFETY: one borrow; nothing inside dispatches.
    let now = unsafe { with_sheet(hwnd, |state| state.geom.zoom) }.unwrap_or(1.0);
    zoom_to(hwnd, now * factor);
}

fn zoom_to(hwnd: HWND, zoom: f64) {
    let zoom = zoom.clamp(0.25, 4.0);
    // SAFETY: one borrow; nothing inside dispatches.
    unsafe {
        with_sheet(hwnd, |state| {
            state.geom.zoom = zoom;
            state.say(Some(format!("Zoom {:.0}%.", zoom * 100.0)));
        });
    }
    refresh(hwnd);
}

/// View ▸ Show Formulas — each formula cell draws its formula instead of its result. A reading:
/// nothing is written, and the menu is rebuilt for the checkmark's sake like the overlays'.
fn toggle_formulas(hwnd: HWND) {
    // SAFETY: one borrow; nothing inside dispatches.
    unsafe {
        with_sheet(hwnd, |state| state.formulas = !state.formulas);
    }
    build_menu(hwnd);
    refresh(hwnd);
}

/// Whether the formula bar shows a formula's friendly reading. The menu is rebuilt straight after,
/// or its check mark would answer with the state before the click.
fn toggle_friendly(hwnd: HWND) {
    // SAFETY: one borrow; nothing inside dispatches.
    unsafe {
        with_sheet(hwnd, |state| state.friendly = !state.friendly);
    }
    build_menu(hwnd);
    refresh(hwnd);
}

/// The function list: every function grind implements, and picking one writes the call.
/// `dialog::choose`'s listbox, rows from `assist::function_lines` (`grind sheet functions
/// --long`'s columns), so window and CLI agree.
fn function_list(hwnd: HWND) {
    let rows = assist::function_lines();
    // SAFETY: one borrow, for one flag; released before the modal, which runs a nested loop.
    let editing = unsafe { with_sheet(hwnd, |state| state.mode.is_editing()) }.unwrap_or(false);
    let Some(at) = dialog::choose(hwnd, "Functions", &rows, 0) else {
        return;
    };
    let Some(insert) = assist::function_insert(at, editing) else {
        return;
    };
    match editing {
        // Into the edit that is already open, at the caret — `EM_REPLACESEL`, the same call an
        // accepted completion uses, so both land one undo step in the control's own history.
        true => {
            let Some((edit, _)) = editor_text(hwnd) else {
                return;
            };
            let wide = gdi::wide(&insert);
            // SAFETY: nothing is borrowed; the buffer is a NUL-terminated local that outlives
            // the call, and both calls dispatch synchronously to this window's own control.
            unsafe {
                let _ = SetFocus(Some(edit));
                SendMessageW(
                    edit,
                    EM_REPLACESEL,
                    Some(WPARAM(1)),
                    Some(LPARAM(wide.as_ptr() as isize)),
                );
            }
            editor_changed(hwnd);
        }
        // Nothing open: this starts the edit, seeded with `=NAME(` and the caret after it, which
        // is where the first argument goes.
        false => begin_edit_with(hwnd, insert, state::Mode::Enter, false),
    }
}

/// The active cell's formula explained — `formula::friendly::explain`, a line a row, read-only:
/// never parsed back, nothing written (R1).
fn explain_formula(hwnd: HWND) {
    // SAFETY: one borrow, released before the modal.
    let Some((address, text)) = (unsafe {
        with_sheet(hwnd, |state| {
            (
                grind_sheet::a1::format(None, state.selection.active),
                status::formula_bar_text(&state.app, state.sheet, state.selection),
            )
        })
    }) else {
        return;
    };
    let Ok(explained) = grind_sheet::formula::friendly::explain(&text) else {
        // Not a formula, or one that will not parse: a sentence in the notice bar, where this
        // window talks about the cell it is on, rather than a modal saying no.
        // SAFETY: one borrow; nothing inside dispatches.
        unsafe {
            with_sheet(hwnd, |state| {
                state.say(Some(notice::nothing_to_explain(&address)));
            });
        }
        refresh(hwnd);
        return;
    };
    let rows: Vec<String> = explained.lines().map(str::to_owned).collect();
    dialog::show_list(hwnd, &format!("{address} explained"), &rows);
}

/// `Pane::project`, spelled so [`with_pane`] can be handed it directly rather than a closure that
/// only repeats the match `Pane::sheet_mut`/`text_mut` already exist to avoid writing twice.
fn project(pane: &mut Pane) -> Option<grind_core::projection::Projection> {
    match pane {
        Pane::Sheet(sheet) => Some(sheet.app.project()),
        Pane::Text(text) => Some(text.app.project()),
        // No document, so nothing to project one into. Not reachable from the welcome screen's own
        // menu bar (`menu::on_welcome`); answered rather than panicked on for a stale accelerator.
        Pane::Welcome(_) => None,
    }
}

/// `Pane::lint`, over the default options — every rule, hints off, the same defaults `grind lint`
/// runs with no flags.
fn lint(pane: &mut Pane) -> Option<grind_core::lint::Report> {
    let options = grind_core::lint::Options::default();
    match pane {
        Pane::Sheet(sheet) => Some(sheet.app.lint(&options)),
        // Spelling is in it, as `misspelt`, whenever a dictionary is attached.
        Pane::Text(text) => Some(text.app.lint(&options)),
        Pane::Welcome(_) => None,
    }
}

/// The address the pane's own selection or caret projects to — what `show_source` marks the
/// source on when it opens.
fn current_address(hwnd: HWND) -> Option<String> {
    if is_text(hwnd) {
        // SAFETY: one borrow; nothing inside dispatches.
        return unsafe { with_text(hwnd, |text| grind_text::loc::format(text.caret.block)) };
    }
    // SAFETY: one borrow; nothing inside dispatches.
    unsafe {
        with_sheet(hwnd, |state| {
            let name = state.app.sheet_name(state.sheet).ok();
            grind_sheet::a1::format(name.as_deref(), state.selection.active)
        })
    }
}

/// Jump either pane to an address Show Source or Check Document produced — [`text_go_to`]'s two
/// calls for a document, `a1::parse`/`a1::resolve` for the grid, **allowed to land on another
/// sheet**, unlike the name box, since a diagnostic may be about any sheet.
fn go_to_address(hwnd: HWND, address: &str) {
    if is_text(hwnd) {
        // SAFETY: a fresh borrow, taken and released within this call — there is no dialog either
        // side of it here, unlike `text_go_to`'s own prompt.
        let outcome = unsafe {
            with_text(hwnd, |text| {
                grind_text::loc::parse(address)
                    .map_err(|e| e.to_string())
                    .and_then(|loc| text.app.resolve_caret(&loc).map_err(|e| e.to_string()))
                    .map(|caret| {
                        text.place(caret, false);
                        text.caret_on = true;
                    })
            })
        };
        match outcome {
            Some(Err(message)) => dialog::error(hwnd, &message),
            Some(Ok(())) => refresh(hwnd),
            None => {}
        }
        return;
    }
    // SAFETY: one borrow.
    let outcome = unsafe {
        with_sheet(hwnd, |state| {
            grind_sheet::a1::parse(address)
                .map_err(|e| e.to_string())
                .and_then(|reference| {
                    grind_sheet::a1::resolve(&state.app, &reference).map_err(|e| e.to_string())
                })
                .map(|(found, start, end)| {
                    state.sheet = found;
                    state.selection = Selection {
                        anchor: end,
                        active: start,
                    };
                })
        })
    };
    match outcome {
        Some(Err(message)) => dialog::error(hwnd, &message),
        Some(Ok(())) => refresh(hwnd),
        None => {}
    }
}

/// The key list — `menu::shortcuts()` in `dialog::choose`'s read-only listbox; the row picked is
/// thrown away, the list being there to read.
fn show_shortcuts(hwnd: HWND) {
    let rows = menu::shortcuts();
    dialog::show_list(hwnd, "Keyboard Shortcuts", &rows);
}

/// F9. The banner reports what happened, including when nothing did — a key that appears to do
/// nothing is a key people press twice.
fn recalculate(hwnd: HWND) {
    // SAFETY: one borrow.
    unsafe {
        with_sheet(hwnd, |state| {
            let said = match state.app.recalc() {
                Ok(recalc) => notice::recalculated(recalc.changed, recalc.spoiled),
                Err(error) => error.to_string(),
            };
            state.say(Some(said));
        });
    }
    refresh(hwnd);
}

/// An autofilter over the selection, or clear the sheet's — Data ▸ Autofilter. Over a sheet with
/// one this clears it; otherwise the selection becomes the range, its first row the heading.
/// Drawing the buttons and folding the hidden rows is `sheet/draw.rs`'s and `relayout`'s.
fn toggle_filter(hwnd: HWND) {
    // SAFETY: one borrow.
    unsafe {
        with_sheet(hwnd, |state| {
            if state.app.filter(state.sheet).unwrap_or(None).is_some() {
                if let Err(error) = state.app.set_filter(state.sheet, None) {
                    state.say(Some(error.to_string()));
                }
                return;
            }
            let (start, end) = state.selection.rect();
            let used = state.app.used_extent(state.sheet).unwrap_or((0, 0));
            match Filter::over_selection(start, end, used) {
                Ok(filter) => {
                    if let Err(error) = state.app.set_filter(state.sheet, Some(filter)) {
                        state.say(Some(error.to_string()));
                    }
                }
                Err(why) => state.say(Some(why.to_owned())),
            }
        });
    }
    refresh(hwnd);
}

/// Open the dropdown for one field, under the button `button_down` caught. Values come from a
/// fresh read of the column's cells from the filter's first data row to its last, deduplicated.
/// Two `with_sheet` calls, since `dialog::choose_multi` runs a nested loop (decision 7).
fn open_filter_menu(hwnd: HWND, col: u32) {
    // SAFETY: one borrow, released before the dialog.
    let Some((field, title, values, checked)) = (unsafe {
        with_sheet(hwnd, |state| {
            let filter = state.app.filter(state.sheet).ok()??;
            let field = col - filter.start.col;
            let first = filter.first_data_row();
            let viewport = state
                .app
                .get_viewport(
                    state.sheet,
                    first..filter.end.row.saturating_add(1),
                    col..col.saturating_add(1),
                )
                .ok()?;
            let mut set = std::collections::BTreeSet::new();
            for row in first..=filter.end.row {
                if let Some(text) = viewport.text(row, col) {
                    set.insert(text.to_owned());
                }
            }
            let values: Vec<String> = set.into_iter().collect();
            let checked: Vec<bool> = match filter.keep.get(&field) {
                Some(keep) => values.iter().map(|v| keep.contains(v)).collect(),
                None => vec![true; values.len()],
            };
            let title = state
                .app
                .get_viewport(
                    state.sheet,
                    filter.start.row..filter.start.row.saturating_add(1),
                    col..col.saturating_add(1),
                )
                .ok()
                .and_then(|v| v.text(filter.start.row, col).map(str::to_owned))
                .filter(|t| !t.is_empty())
                .unwrap_or_else(|| format!("Column {}", field + 1));
            Some((field, title, values, checked))
        })
    })
    .flatten() else {
        return;
    };
    if values.is_empty() {
        return;
    }
    let Some(choice) = dialog::choose_multi(hwnd, &title, &values, &checked) else {
        return;
    };
    // SAFETY: a fresh borrow, taken after the dialog rather than across it.
    unsafe {
        with_sheet(hwnd, |state| {
            let Ok(Some(mut filter)) = state.app.filter(state.sheet) else {
                return;
            };
            match choice {
                dialog::FilterChoice::Clear => {
                    filter.keep.remove(&field);
                }
                dialog::FilterChoice::Keep(chosen) => {
                    let keep = values
                        .iter()
                        .zip(chosen)
                        .filter(|(_, kept)| *kept)
                        .map(|(v, _)| v.clone())
                        .collect();
                    filter.keep.insert(field, keep);
                }
            }
            if let Err(error) = state.app.set_filter(state.sheet, Some(filter)) {
                state.say(Some(error.to_string()));
            }
        });
    }
    refresh(hwnd);
}

/// Format the selection as a table — Data ▸ Format as Table. No dialog: the selection is the
/// range, its first row the heading, the name `Table1`, `Table2`, …. No un-format — ODF keeps no
/// table object — so its effects go by Undo (one step) or by hand, as after LibreOffice's
/// AutoFormat (`sheet/src/table_format.rs`).
fn format_table(hwnd: HWND, totals: Option<grind_sheet::TotalsFunction>) {
    // SAFETY: one borrow.
    unsafe {
        with_sheet(hwnd, |state| {
            let (start, mut end) = state.selection.rect();
            // A single cell is a click, not a range — the same rule `sheet.filter` uses.
            if start == end
                && let Ok((rows, cols)) = state.app.used_extent(state.sheet)
            {
                end = Pos::new(rows.saturating_sub(1), cols.saturating_sub(1));
            }
            if end.row <= start.row {
                return state.say(Some(
                    "Select the rows to format, including their headings".to_owned(),
                ));
            }
            let options = TableOptions {
                header: true,
                totals,
                name: None,
            };
            if let Err(error) = state.app.format_table(state.sheet, start, end, options) {
                state.say(Some(error.to_string()));
            }
        });
    }
    refresh(hwnd);
}

/// Format the selection as `numfmt::CURRENCIES[index]` — the Format menu's three items and the
/// cells' context menu. `sheet/currency.rs` decides: a currency cell keeps its decimals, grouping
/// and locale and changes its symbol; anything else gets the defaults. One `App::set_format`, one
/// undo step.
fn set_currency(hwnd: HWND, index: usize) {
    let Some((symbol, _)) = grind_sheet::numfmt::CURRENCIES.get(index) else {
        return;
    };
    // SAFETY: one borrow; nothing inside dispatches.
    unsafe {
        with_sheet(hwnd, |state| {
            let used = state.app.used_extent(state.sheet).unwrap_or((0, 0));
            let (start, end) = grind_sheet::nav::target(state.selection, used);
            let current = state
                .app
                .format_at(state.sheet, state.selection.active)
                .ok()
                .flatten();
            let format = currency::format_for(
                current.as_ref(),
                symbol,
                grind_sheet::locale::from_environment(),
            );
            if let Err(error) = state.app.set_format(state.sheet, start, end, Some(format)) {
                state.say(Some(error.to_string()));
            }
        });
    }
    refresh(hwnd);
}

/// What one format-strip control does once pressed and released over itself — and the same verb
/// from the Format menu or a key. Every write is one `App::set_style` or `App::set_format` over
/// the selection cut to the sheet in use (`grind_sheet::nav::target`), one Ctrl+Z; *Clear* makes
/// two calls, the core having one "plain again" per kind of property.
fn format_control(hwnd: HWND, control: format::Control) {
    use format::Control;
    match control {
        Control::Color => format_pick_color(hwnd, false),
        Control::Background => format_pick_color(hwnd, true),
        Control::Number => format_pick_number(hwnd),
        Control::Outdent => format_write(hwnd, |state, start, end| {
            let style = grind_sheet::format::indented(&active_style(state), -1);
            state.app.set_style(state.sheet, start, end, style)
        }),
        Control::Indent => format_write(hwnd, |state, start, end| {
            let style = grind_sheet::format::indented(&active_style(state), 1);
            state.app.set_style(state.sheet, start, end, style)
        }),
        Control::FewerDecimals => format_step_decimals(hwnd, -1),
        Control::MoreDecimals => format_step_decimals(hwnd, 1),
        Control::Clear => format_write(hwnd, |state, start, end| {
            state.app.set_style(state.sheet, start, end, None)?;
            state.app.set_format(state.sheet, start, end, None)
        }),
        toggle => format_write(hwnd, |state, start, end| {
            let style = active_style(state);
            match format::toggled(&style, toggle) {
                Some(written) => state.app.set_style(state.sheet, start, end, written),
                None => Ok(0),
            }
        }),
    }
}

/// Run one formatting write over the selection's target rectangle, and say so in the notice bar
/// when the core refuses it — a million-cell request the target did not cut, say — rather than
/// doing nothing where somebody can see nothing happened.
fn format_write(
    hwnd: HWND,
    write: impl FnOnce(&mut Sheet, Pos, Pos) -> grind_sheet::Result<usize>,
) {
    // SAFETY: one borrow; the core's writes notify, and the observer posts rather than sends.
    unsafe {
        with_sheet(hwnd, |state| {
            let used = state.app.used_extent(state.sheet).unwrap_or((0, 0));
            let (start, end) = grind_sheet::nav::target(state.selection, used);
            match write(state, start, end) {
                Ok(_) => state.say(None),
                Err(error) => state.say(Some(error.to_string())),
            }
        });
    }
    refresh(hwnd);
}

/// The active cell's own style, or the plain one — what every toggle reads before it writes.
fn active_style(state: &Sheet) -> grind_sheet::style::CellStyle {
    state
        .app
        .style_at(state.sheet, state.selection.active)
        .ok()
        .flatten()
        .unwrap_or_default()
}

/// *Text Colour* and *Cell Background* — `dialog::choose` over *Automatic* and
/// `grind_core::style::PALETTE`, the text pane's own picker and the same palette every shell and
/// `grind sheet style --color` offer, opening on the active cell's own colour.
fn format_pick_color(hwnd: HWND, background: bool) {
    // SAFETY: one borrow, released before the chooser's nested loop.
    let Some(style) = (unsafe { with_sheet(hwnd, |state| active_style(state)) }) else {
        return;
    };
    let current = match background {
        true => style.background.as_deref(),
        false => style.color.as_deref(),
    };
    let items = crate::strip::colour_choices();
    let initial = crate::strip::colour_row(current);
    let title = match background {
        true => format::Control::Background.name(),
        false => format::Control::Color.name(),
    };
    let Some(choice) = dialog::choose(hwnd, title, &items, initial) else {
        return;
    };
    let value = (choice != 0).then(|| grind_core::style::PALETTE[choice - 1].1.to_owned());
    format_write(hwnd, |state, start, end| {
        let written = format::coloured(&active_style(state), background, value);
        state.app.set_style(state.sheet, start, end, written)
    });
}

/// *Number Format* — `dialog::choose` over `format::KINDS`, opening on the row the active cell
/// already is, and one click is the whole request (`format::format_for_kind`).
/// *Text Rotation…* — one of `format::ROTATIONS`, the active cell's checked first.
fn format_pick_rotation(hwnd: HWND) {
    use grind_sheet::format::{ROTATIONS, rotated, rotation_chosen};
    // SAFETY: one borrow, released before the chooser's nested loop.
    let Some(initial) =
        (unsafe { with_sheet(hwnd, |state| rotation_chosen(&active_style(state))) })
    else {
        return;
    };
    let items: Vec<String> = ROTATIONS
        .iter()
        .map(|(_, name)| (*name).to_owned())
        .collect();
    let Some(choice) = dialog::choose(hwnd, "Text Rotation", &items, initial.unwrap_or(0)) else {
        return;
    };
    format_write(hwnd, |state, start, end| {
        let style = rotated(&active_style(state), ROTATIONS[choice].0);
        state.app.set_style(state.sheet, start, end, style)
    });
}

fn format_pick_number(hwnd: HWND) {
    // SAFETY: one borrow, released before the chooser's nested loop.
    let Some((current, items)) = (unsafe {
        with_sheet(hwnd, |state| {
            let at = state.selection.active;
            let current = state.app.format_at(state.sheet, at).ok().flatten();
            let locale = grind_sheet::locale::from_environment();
            // The live sample: what the cell would show under each row, `App::shown_as` and
            // nothing written — the line the GNOME popover and the Mac's menu items carry.
            let items: Vec<String> = format::KINDS
                .iter()
                .enumerate()
                .map(|(index, (label, _))| {
                    let format = format::format_for_kind(current.as_ref(), index, locale.clone());
                    let sample = state.app.shown_as(state.sheet, at, format.as_ref()).ok();
                    format::row_label(label, sample.as_deref())
                })
                .collect();
            (current, items)
        })
    }) else {
        return;
    };
    let initial = format::kind_of(current.as_ref()).unwrap_or(0);
    let Some(choice) = dialog::choose(hwnd, format::Control::Number.name(), &items, initial) else {
        return;
    };
    format_write(hwnd, |state, start, end| {
        let current = state
            .app
            .format_at(state.sheet, state.selection.active)
            .ok()
            .flatten();
        let written = format::format_for_kind(
            current.as_ref(),
            choice,
            grind_sheet::locale::from_environment(),
        );
        state.app.set_format(state.sheet, start, end, written)
    });
}

/// One decimal more or fewer — `format::stepped`, starting a plain cell from the decimals it
/// *shows*, read off the viewport the way a renderer reads it.
/// Format ▸ Use 1000 Separator — [`format_step_decimals`]'s shape, over `format::grouped`.
fn format_group_thousands(hwnd: HWND) {
    format_write(hwnd, |state, start, end| {
        let active = state.selection.active;
        let current = state.app.format_at(state.sheet, active).ok().flatten();
        let shown = state
            .app
            .get_viewport(
                state.sheet,
                active.row..active.row + 1,
                active.col..active.col + 1,
            )
            .ok()
            .and_then(|view| {
                view.text(active.row, active.col)
                    .map(|text| format::decimals_shown(text, state.app.locale().as_ref()))
            })
            .unwrap_or(0);
        match grind_sheet::format::grouped(
            current.as_ref(),
            shown,
            grind_sheet::locale::from_environment(),
        ) {
            Some(written) => state.app.set_format(state.sheet, start, end, Some(written)),
            None => Ok(0),
        }
    });
}

fn format_step_decimals(hwnd: HWND, step: i8) {
    format_write(hwnd, |state, start, end| {
        let active = state.selection.active;
        let current = state.app.format_at(state.sheet, active).ok().flatten();
        let shown = state
            .app
            .get_viewport(
                state.sheet,
                active.row..active.row + 1,
                active.col..active.col + 1,
            )
            .ok()
            .and_then(|view| {
                view.text(active.row, active.col)
                    .map(|text| format::decimals_shown(text, state.app.locale().as_ref()))
            })
            .unwrap_or(0);
        match format::stepped(
            current.as_ref(),
            step,
            shown,
            grind_sheet::locale::from_environment(),
        ) {
            Some(written) => state.app.set_format(state.sheet, start, end, Some(written)),
            None => Ok(0),
        }
    });
}

/// Check the *Change to …* item naming the kind the chart those items act on already is — the
/// selected chart, else the last — in whichever menu is opening, `check_format`'s arrangement.
fn check_chart_kind(hwnd: HWND, popup: HMENU) {
    // SAFETY: one borrow; nothing inside dispatches. `None` off the grid.
    let Some(kind) = (unsafe {
        with_sheet(hwnd, |state| {
            let index = state.chart_target()?;
            state
                .app
                .charts(state.sheet)
                .ok()?
                .get(index)
                .map(|chart| chart.kind)
        })
    }) else {
        return;
    };
    for (command, each) in Command::CHART_KINDS
        .into_iter()
        .zip(grind_sheet::ChartKind::ALL)
    {
        let flag = match kind == Some(each) {
            true => MF_CHECKED,
            false => MF_UNCHECKED,
        };
        // SAFETY: `popup` is the menu Windows is about to show, alive for this message.
        unsafe {
            let _ = CheckMenuItem(popup, u32::from(command.id()), (MF_BYCOMMAND | flag).0);
        }
    }
}

/// Check the format items describing the active cell — its currency and the strip's pressed
/// toggles — and uncheck the rest, in whichever menu is opening. `MF_BYCOMMAND` on a popup holding
/// none of them finds nothing, so this needs no idea which menu it is.
fn check_format(hwnd: HWND, popup: HMENU) {
    // SAFETY: one borrow; nothing inside dispatches. `None` off the grid, where no currency item
    // is ever shown.
    let Some((chosen, grouped)) = (unsafe {
        with_sheet(hwnd, |state| {
            let current = state
                .app
                .format_at(state.sheet, state.selection.active)
                .ok()
                .flatten();
            (
                currency::chosen(current.as_ref()),
                grind_sheet::format::is_grouped(current.as_ref()),
            )
        })
    }) else {
        return;
    };
    let check = |command: Command, on: bool| {
        let flag = match on {
            true => MF_CHECKED,
            false => MF_UNCHECKED,
        };
        // SAFETY: `popup` is the menu Windows is about to show, alive for this message.
        unsafe {
            let _ = CheckMenuItem(popup, u32::from(command.id()), (MF_BYCOMMAND | flag).0);
        }
    };
    for (index, command) in Command::CURRENCIES.iter().enumerate() {
        check(*command, chosen == Some(index));
    }
    check(Command::GroupThousands, grouped);
    // The strip's five toggles, checked as the strip draws them pressed — so the menu and the strip
    // cannot disagree about whether the active cell is bold.
    // SAFETY: one borrow; nothing inside dispatches.
    let Some(style) = (unsafe { with_sheet(hwnd, |state| active_style(state)) }) else {
        return;
    };
    for (command, control) in [
        (Command::Bold, format::Control::Bold),
        (Command::Italic, format::Control::Italic),
        (Command::Underline, format::Control::Underline),
        (Command::Strike, format::Control::Strike),
        (Command::AlignLeft, format::Control::AlignLeft),
        (Command::AlignCenter, format::Control::AlignCenter),
        (Command::AlignRight, format::Control::AlignRight),
    ] {
        check(command, format::pressed(&style, control));
    }
}

/// *Format as Table with Totals…* — [`format_table`], having asked which aggregate the totals row
/// carries, in `dialog::choose` over `TotalsFunction::label`s, so the aggregates have the names
/// every client gives them. Chosen outside `with_sheet`, as the chooser runs a nested loop
/// (decision 7).
fn format_table_totals(hwnd: HWND) {
    let rows: Vec<String> = grind_sheet::TotalsFunction::ALL
        .iter()
        .map(|f| f.label().to_owned())
        .collect();
    let Some(picked) = dialog::choose(hwnd, "Totals row", &rows, 0) else {
        return;
    };
    let Some(function) = grind_sheet::TotalsFunction::ALL.get(picked).copied() else {
        return;
    };
    format_table(hwnd, Some(function));
}

// --- conditional formatting (`doc/conditional-format.md` §4) ---

/// Data ▸ Add Conditional Format… — the condition in a prompt, the look in a chooser, then
/// `grind_sheet::rule::add_from_input` over the selection. Both dialogs run nested loops, so the
/// selection is read in one borrow and the rule written in another (decision 7).
fn add_rule(hwnd: HWND) {
    use grind_sheet::rule::{self, Look};
    // SAFETY: one borrow, released before the prompt.
    let Some((sheet, start, end)) = (unsafe {
        with_sheet(hwnd, |state| {
            let (start, end) = state.selection.rect();
            (state.sheet, start, end)
        })
    }) else {
        return;
    };
    let range = rule::range_hint(start, end);
    let Some(condition) = dialog::prompt(
        hwnd,
        "Add Conditional Format",
        &format!(
            "Draw {range} differently where this formula is true, written for its first cell:"
        ),
        &rule::condition_hint(start),
    ) else {
        return;
    };
    let looks: Vec<String> = Look::ALL
        .iter()
        .map(|look| look.label().to_owned())
        .collect();
    let Some(look) = dialog::choose(hwnd, "Draw It With", &looks, 0)
        .and_then(|picked| Look::ALL.get(picked).copied())
    else {
        return;
    };
    // SAFETY: a fresh borrow, after both dialogs.
    unsafe {
        with_sheet(hwnd, |state| {
            state.say(Some(
                match rule::add_from_input(&state.app, sheet, &range, &condition, look) {
                    Ok((_, index)) => notice::rule_added(index, look.label()),
                    Err(error) => error,
                },
            ));
        });
    }
    refresh(hwnd);
}

/// Data ▸ Remove Conditional Format… — the sheet's rules in a chooser, in the order they are
/// tried, the first touching the selection picked; the one chosen is taken away.
fn remove_rule(hwnd: HWND) {
    // SAFETY: one borrow, released before the chooser.
    let Some((sheet, rows, initial)) = (unsafe {
        with_sheet(hwnd, |state| {
            let (start, end) = state.selection.rect();
            let rules = state.app.rules(state.sheet).unwrap_or_default();
            let rows: Vec<String> = rules
                .iter()
                .enumerate()
                .map(|(index, r)| {
                    format!(
                        "{}.  {}",
                        index + 1,
                        grind_sheet::rule::summary(r).replace('\t', "    ")
                    )
                })
                .collect();
            let initial = rules
                .iter()
                .position(|r| r.touches(start, end))
                .unwrap_or(0);
            (state.sheet, rows, initial)
        })
    }) else {
        return;
    };
    if rows.is_empty() {
        // SAFETY: a fresh borrow.
        unsafe {
            with_sheet(hwnd, |state| state.say(Some(notice::no_rules())));
        }
        return refresh(hwnd);
    }
    let Some(picked) = dialog::choose(hwnd, "Remove Conditional Format", &rows, initial) else {
        return;
    };
    // SAFETY: a fresh borrow, after the chooser.
    unsafe {
        with_sheet(hwnd, |state| {
            state.say(Some(match state.app.remove_rule(sheet, picked) {
                Ok(_) => notice::rule_removed(picked),
                Err(error) => error.to_string(),
            }));
        });
    }
    refresh(hwnd);
}

// --- CSV, the one non-ODF format (`doc/not-doing.md` §2) ---

/// Edit ▸ Find… — ask for a word, then land on the first cell holding it at or after the cursor,
/// across every sheet. The prompt runs a nested loop, so the word offered is read in one borrow
/// and the search is a second (decision 7).
fn find(hwnd: HWND) {
    // SAFETY: one borrow, released before the prompt.
    let Some(offered) = (unsafe { with_sheet(hwnd, |state| state.needle.clone()) }) else {
        return;
    };
    let Some(needle) = dialog::prompt(hwnd, "Find", "Find in every sheet:", &offered) else {
        return;
    };
    if needle.is_empty() {
        return;
    }
    // SAFETY: a fresh borrow, taken after the dialog has closed.
    unsafe {
        with_sheet(hwnd, |state| state.needle = needle);
    }
    find_step(hwnd, find::Towards::Here);
}

/// F3 / Shift+F3, and Find…'s landing: `App::find` for the remembered word and
/// `grind_sheet::find::step` for the next hit, asked again each time since the document may have
/// changed.
fn find_step(hwnd: HWND, towards: find::Towards) {
    // SAFETY: one borrow, and no dialog on either side of it.
    let asked = unsafe {
        with_sheet(hwnd, |state| {
            if state.needle.is_empty() {
                return false;
            }
            let hits: Vec<(usize, Pos)> = state
                .app
                .find(&find::Search::new(state.needle.as_str()))
                .map(|hits| hits.into_iter().map(|hit| (hit.sheet, hit.pos)).collect())
                .unwrap_or_default();
            let here = (state.sheet, state.selection.active);
            let said = match find::step(&hits, here, towards) {
                Some(index) => {
                    let (sheet, pos) = hits[index];
                    state.sheet = sheet;
                    state.selection = Selection::at(pos);
                    notice::found(index, hits.len(), &state.needle)
                }
                None => notice::not_found(&state.needle),
            };
            state.say(Some(said));
            true
        })
    };
    match asked {
        // Nothing searched for yet: F3 means "find", which is where the word comes from.
        Some(false) => find(hwnd),
        Some(true) => refresh(hwnd),
        None => {}
    }
}

/// F7 — select the next misspelt word after the caret, wrapping as F3 does
/// (`grind_text::find::step`), and open the spelling popup on it.
fn text_next_misspelling(hwnd: HWND) {
    // SAFETY: one borrow, released before the popup's nested message loop.
    let found = unsafe {
        with_text(hwnd, |text| {
            let Some(choice) = text.checked else {
                text.say(Some(notice::spelling_off()));
                return None;
            };
            let all = text.app.misspellings(0..usize::MAX);
            let carets: Vec<Caret> = all.iter().map(|m| m.caret()).collect();
            let (_, at) = text::keymap::ordered(text.anchor, text.caret);
            let Some(index) = grind_text::find::step(&carets, at, grind_text::find::Towards::Next)
            else {
                text.say(Some(notice::none_misspelt(&choice.describe())));
                return None;
            };
            let wrong = all[index].clone();
            text.place(wrong.caret(), false);
            text.place(
                Caret {
                    block: wrong.block,
                    offset: wrong.offset + wrong.len,
                },
                true,
            );
            text.caret_on = true;
            text.say(Some(notice::misspelt(index, all.len(), &wrong.word)));
            Some(wrong)
        })
    }
    .flatten();
    text_refresh(hwnd);
    let Some(wrong) = found else {
        return;
    };
    // Under the word, where a right click on it would have opened the same popup.
    let geometry = unsafe { with_text(hwnd, |text| text.caret_geometry()) }.flatten();
    let mut at = match geometry {
        Some((x, y, _, height)) => POINT { x, y: y + height },
        None => POINT { x: 0, y: 0 },
    };
    // SAFETY: `hwnd` is this window's and `at` is a live local.
    unsafe {
        let _ = ClientToScreen(hwnd, &mut at);
    }
    spelling_popup(hwnd, at, Some(wrong), &[]);
}

/// View ▸ Spelling Language… — Automatic, a dictionary by name, or Off, in the listbox every
/// other chooser here uses. Remembered for every text document; the document's own language is
/// read and never written (`doc/spelling.md`).
fn text_spelling_language(hwnd: HWND) {
    // SAFETY: one borrow, released before the dialog.
    let Some(current) = (unsafe { with_text(hwnd, |text| text.spelling) }) else {
        return;
    };
    let settings = grind_spell::Setting::ALL;
    let items: Vec<String> = settings.iter().map(|s| s.label().to_owned()).collect();
    let selected = settings.iter().position(|s| *s == current).unwrap_or(0);
    let Some(choice) = dialog::choose(hwnd, "Spelling Language", &items, selected) else {
        return;
    };
    // SAFETY: a fresh borrow, taken after the dialog has closed.
    unsafe {
        with_text(hwnd, |text| text.set_spelling(settings[choice]));
    }
    text_refresh(hwnd);
}

/// What somebody picked from a spelling popup.
enum SpellPick {
    Nothing,
    Correct(String),
    Ignore,
    Add,
    Command(Command),
}

/// A popup with a misspelt word's suggestions, Ignore All and Add to Dictionary on top and
/// `commands` under — both panes' context menus and F7's popup, so a misspelling gets the same
/// rows everywhere. `word` is `None` over a word spelled right, leaving only the commands. A
/// nested loop (decision 7): suggestions are asked for before, the pick acted on after.
fn spelling_menu(
    hwnd: HWND,
    at: POINT,
    word: Option<&str>,
    offers: &[String],
    commands: &[Command],
) -> SpellPick {
    let ignore = menu::SPELLING_FIRST_ID + menu::SPELLING_IDS - 2;
    let add = ignore + 1;
    // SAFETY: the popup is built and destroyed within this call, and `TrackPopupMenuEx` is the
    // one nested message loop in it — nothing is borrowed across it.
    let picked = unsafe {
        let Ok(popup) = CreatePopupMenu() else {
            return SpellPick::Nothing;
        };
        let append = |flags, id: u16, label: &str| {
            let label = gdi::wide(label);
            let _ = AppendMenuW(popup, flags, usize::from(id), PCWSTR(label.as_ptr()));
        };
        if word.is_some() {
            for (i, word) in offers.iter().enumerate() {
                append(MF_STRING, menu::SPELLING_FIRST_ID + i as u16, word);
            }
            if offers.is_empty() {
                append(MF_STRING | MF_GRAYED, 0, "No Suggestions");
            }
            let _ = AppendMenuW(popup, MF_SEPARATOR, 0, PCWSTR::null());
            append(MF_STRING, ignore, "&Ignore All");
            append(MF_STRING, add, "&Add to Dictionary");
            if !commands.is_empty() {
                let _ = AppendMenuW(popup, MF_SEPARATOR, 0, PCWSTR::null());
            }
        }
        for command in commands {
            // The currencies are a group of their own, as they are in the Format menu.
            if command.currency() == Some(0) {
                let _ = AppendMenuW(popup, MF_SEPARATOR, 0, PCWSTR::null());
            }
            if let Some(label) = menu::label_for(*command) {
                append(MF_STRING, command.id(), label);
            }
        }
        let result = TrackPopupMenuEx(
            popup,
            (TPM_RETURNCMD | TPM_RIGHTBUTTON).0,
            at.x,
            at.y,
            hwnd,
            None,
        );
        let _ = DestroyMenu(popup);
        result.0 as u16
    };
    match picked {
        0 => SpellPick::Nothing,
        id if word.is_some() && id == ignore => SpellPick::Ignore,
        id if word.is_some() && id == add => SpellPick::Add,
        id if word.is_some() && (menu::SPELLING_FIRST_ID..ignore).contains(&id) => offers
            .get(usize::from(id - menu::SPELLING_FIRST_ID))
            .map_or(SpellPick::Nothing, |with| SpellPick::Correct(with.clone())),
        id => menu::command_for(id).map_or(SpellPick::Nothing, SpellPick::Command),
    }
}

/// The suggestions a popup offers for `word`, as many as it has rows for.
fn offers_for(suggestions: Vec<String>) -> Vec<String> {
    suggestions
        .into_iter()
        .take(usize::from(menu::SPELLING_IDS - 2))
        .collect()
}

/// Accept `word` into `speller` — for this session, and with `add` in the person's own list
/// too — and the sentence the notice bar says about it.
fn accept_word(speller: Option<&grind_spell::Speller>, word: &str, add: bool) -> Option<String> {
    let speller = speller?;
    speller.accept(word);
    Some(match add {
        false => notice::accepted(word, None),
        true => match grind_spell::personal::add(word) {
            Ok(path) => notice::accepted(word, Some(&path.display().to_string())),
            Err(error) => error.to_string(),
        },
    })
}

/// The text pane's spelling popup: [`spelling_menu`] over a misspelt word in a paragraph.
fn spelling_popup(
    hwnd: HWND,
    at: POINT,
    wrong: Option<grind_text::Misspelling>,
    commands: &[Command],
) {
    // Asked before the popup opens: tens of milliseconds, and the core is not to be borrowed
    // across a nested message loop (decision 7).
    let offers: Vec<String> = match &wrong {
        // SAFETY: one borrow; `suggest` dispatches nothing.
        Some(wrong) => offers_for(
            unsafe { with_text(hwnd, |text| text.app.suggest(&wrong.word)) }.unwrap_or_default(),
        ),
        None => Vec::new(),
    };
    let picked = spelling_menu(
        hwnd,
        at,
        wrong.as_ref().map(|w| w.word.as_str()),
        &offers,
        commands,
    );
    match (picked, wrong) {
        (SpellPick::Command(command), _) => do_command(hwnd, command),
        (pick @ (SpellPick::Ignore | SpellPick::Add), Some(wrong)) => {
            // SAFETY: a fresh borrow, taken after the popup has closed.
            unsafe {
                with_text(hwnd, |text| {
                    let add = matches!(pick, SpellPick::Add);
                    if let Some(said) = accept_word(text.speller.as_deref(), &wrong.word, add) {
                        text.say(Some(said));
                    }
                });
            }
            text_refresh(hwnd);
        }
        (SpellPick::Correct(with), Some(wrong)) => {
            // SAFETY: a fresh borrow, taken after the popup has closed.
            unsafe {
                with_text(hwnd, |text| {
                    match text.app.correct(wrong.caret(), &wrong.word, &with) {
                        Ok(()) => {
                            let end = Caret {
                                block: wrong.block,
                                offset: wrong.offset + with.chars().count(),
                            };
                            text.place(end, false);
                            text.say(None);
                        }
                        Err(error) => text.say(Some(error.to_string())),
                    }
                });
            }
            text_refresh(hwnd);
        }
        _ => {}
    }
}

/// The grid's spelling popup: [`spelling_menu`] over a misspelt word in a text cell, with the
/// cell menu's `commands` under it.
fn sheet_spelling_popup(
    hwnd: HWND,
    at: POINT,
    wrong: Option<grind_sheet::Misspelling>,
    commands: &[Command],
) {
    let offers: Vec<String> = match &wrong {
        // SAFETY: one borrow; `suggest` dispatches nothing.
        Some(wrong) => offers_for(
            unsafe { with_sheet(hwnd, |state| state.app.suggest(&wrong.word)) }.unwrap_or_default(),
        ),
        None => Vec::new(),
    };
    let picked = spelling_menu(
        hwnd,
        at,
        wrong.as_ref().map(|w| w.word.as_str()),
        &offers,
        commands,
    );
    match (picked, wrong) {
        (SpellPick::Command(command), _) => do_command(hwnd, command),
        (pick @ (SpellPick::Ignore | SpellPick::Add), Some(wrong)) => {
            // SAFETY: a fresh borrow, taken after the popup has closed.
            unsafe {
                with_sheet(hwnd, |state| {
                    let add = matches!(pick, SpellPick::Add);
                    if let Some(said) = accept_word(state.speller.as_deref(), &wrong.word, add) {
                        state.say(Some(said));
                    }
                    state.spell_stop = None;
                });
            }
            refresh(hwnd);
        }
        (SpellPick::Correct(with), Some(wrong)) => {
            // SAFETY: a fresh borrow, taken after the popup has closed. `correct` notifies, and
            // the observer posts rather than sends.
            unsafe {
                with_sheet(hwnd, |state| {
                    match state.app.correct(
                        wrong.sheet,
                        wrong.pos,
                        wrong.offset,
                        &wrong.word,
                        &with,
                    ) {
                        Ok(_) => {
                            state.dirty = true;
                            state.spell_stop = None;
                            state.say(None);
                        }
                        Err(error) => state.say(Some(error.to_string())),
                    }
                });
            }
            refresh(hwnd);
        }
        _ => {}
    }
}

/// F7 over the grid — the next misspelt word in reading order, across every sheet and wrapping
/// as F3 does: its cell selected, and the spelling popup opened under it.
fn sheet_next_misspelling(hwnd: HWND) {
    // SAFETY: one borrow, released before the popup's nested message loop.
    let found = unsafe {
        with_sheet(hwnd, |state| {
            let Some(choice) = state.checked else {
                state.say(Some(notice::spelling_off()));
                return None;
            };
            let all = state.app.misspellings(None, None).unwrap_or_default();
            if all.is_empty() {
                state.say(Some(notice::none_misspelt(&choice.describe())));
                return None;
            }
            let key = |m: &grind_sheet::Misspelling| (m.sheet, m.pos.row, m.pos.col, m.offset);
            let active = state.selection.active;
            // From the word F7 last stopped on while its cell is still the active one, else from
            // just before the active cell, so a cell's own first word is the next one.
            let after = match &state.spell_stop {
                Some(stop) if stop.sheet == state.sheet && stop.pos == active => key(stop),
                _ => (state.sheet, active.row, active.col, usize::MAX),
            };
            let fresh = !matches!(&state.spell_stop,
                Some(stop) if stop.sheet == state.sheet && stop.pos == active);
            let index = all
                .iter()
                .position(|m| {
                    let k = key(m);
                    match fresh {
                        true => (k.0, k.1, k.2) >= (after.0, after.1, after.2),
                        false => k > after,
                    }
                })
                .unwrap_or(0);
            let wrong = all[index].clone();
            state.sheet = wrong.sheet;
            state.selection = Selection::at(wrong.pos);
            state.spell_stop = Some(wrong.clone());
            state.say(Some(notice::misspelt(index, all.len(), &wrong.word)));
            Some(wrong)
        })
    }
    .flatten();
    refresh(hwnd);
    let Some(wrong) = found else {
        return;
    };
    // Under the cell, where a right click on it would have opened the same popup.
    // SAFETY: one borrow; nothing inside dispatches.
    let cell = unsafe {
        with_sheet(hwnd, |state| {
            state.geom.cell_rect(wrong.pos.row, wrong.pos.col)
        })
    };
    let mut at = match cell {
        Some(rect) => POINT {
            x: rect.x.round() as i32,
            y: (rect.y + rect.h).round() as i32,
        },
        None => POINT { x: 0, y: 0 },
    };
    // SAFETY: `hwnd` is this window's and `at` is a live local.
    unsafe {
        let _ = ClientToScreen(hwnd, &mut at);
    }
    sheet_spelling_popup(hwnd, at, Some(wrong), &[]);
}

/// View ▸ Spelling Language… over the grid — Automatic, a dictionary by name, or Off, in the
/// listbox every chooser here uses — remembered for every spreadsheet.
fn sheet_spelling_language(hwnd: HWND) {
    // SAFETY: one borrow, released before the dialog.
    let Some(current) = (unsafe { with_sheet(hwnd, |state| state.spelling) }) else {
        return;
    };
    let settings = grind_spell::Setting::ALL;
    let items: Vec<String> = settings.iter().map(|s| s.label().to_owned()).collect();
    let selected = settings.iter().position(|s| *s == current).unwrap_or(0);
    let Some(choice) = dialog::choose(hwnd, "Spelling Language", &items, selected) else {
        return;
    };
    // SAFETY: a fresh borrow, taken after the dialog has closed.
    unsafe {
        with_sheet(hwnd, |state| state.set_spelling(settings[choice]));
    }
    build_menu(hwnd);
    refresh(hwnd);
}

/// View ▸ Personal Dictionary… over either pane: the list in `dialog::choose`, whose first row
/// adds a word and each other row removes its word — asked again after each change, until Cancel.
/// The running speller is told at once (`Speller::accept`/`forget`).
fn personal_dictionary(hwnd: HWND) {
    use grind_spell::personal;
    let tell = |word: &str, add: bool| {
        let act = |speller: &Option<std::sync::Arc<grind_spell::Speller>>| {
            if let Some(speller) = speller {
                match add {
                    true => speller.accept(word),
                    false => speller.forget(word),
                }
            }
        };
        // SAFETY: one borrow each; nothing inside dispatches.
        unsafe {
            let _ = with_sheet(hwnd, |state| act(&state.speller));
            let _ = with_text(hwnd, |state| act(&state.speller));
        }
    };
    let mut said = None;
    loop {
        let words = personal::words();
        let mut items = vec!["Add a word…".to_owned()];
        items.extend(
            words
                .iter()
                .map(|word| format!("Remove \u{201c}{word}\u{201d}")),
        );
        let title = match &said {
            Some(said) => format!("Personal Dictionary \u{2014} {said}"),
            None => "Personal Dictionary".to_owned(),
        };
        let Some(picked) = dialog::choose(hwnd, &title, &items, 0) else {
            break;
        };
        said = Some(match picked {
            0 => {
                let Some(typed) = dialog::prompt(hwnd, "Add a Word", "Word", "") else {
                    continue;
                };
                match personal::word(&typed)
                    .map_err(str::to_owned)
                    .and_then(|word| {
                        personal::add(&word)
                            .map(|_| word)
                            .map_err(|e| e.to_string())
                    }) {
                    Ok(word) => {
                        tell(&word, true);
                        format!("{word} added")
                    }
                    Err(why) => why,
                }
            }
            n => {
                let word = &words[n - 1];
                match personal::remove(word) {
                    Ok(_) => {
                        tell(word, false);
                        format!("{word} removed")
                    }
                    Err(why) => why.to_string(),
                }
            }
        });
        refresh(hwnd);
    }
    refresh(hwnd);
}

/// View ▸ Check Spelling (Shift+F7) over the grid: off, or back on to the language it was last
/// on in — remembered, so off here is off in every spreadsheet until it is turned back on.
fn sheet_toggle_spelling(hwnd: HWND) {
    // SAFETY: one borrow; nothing inside dispatches.
    unsafe {
        with_sheet(hwnd, |state| {
            let next = match state.spelling {
                grind_spell::Setting::Off => state.spelling_on,
                _ => grind_spell::Setting::Off,
            };
            state.set_spelling(next);
            if next == grind_spell::Setting::Off {
                state.say(Some(notice::spelling_turned_off()));
            }
        });
    }
    build_menu(hwnd);
    refresh(hwnd);
}

/// View ▸ Check Spelling (Shift+F7) over a text document: off, or back on — the session's, as
/// the text pane's Spelling Language… is.
fn text_toggle_spelling(hwnd: HWND) {
    // SAFETY: one borrow; nothing inside dispatches.
    unsafe {
        with_text(hwnd, |text| {
            let next = match text.spelling {
                grind_spell::Setting::Off => grind_spell::Setting::Automatic,
                _ => grind_spell::Setting::Off,
            };
            text.set_spelling(next);
        });
    }
    build_menu(hwnd);
    text_refresh(hwnd);
}

/// Edit ▸ Replace… — what, then with what, then `App::replace` over every sheet in one undo
/// step. Two prompts rather than a dialog of its own, since `dialog::prompt` is the one text
/// question this shell has and a replace is two words; neither is asked with anything borrowed.
fn replace(hwnd: HWND) {
    // SAFETY: one borrow, released before the prompts.
    let Some(offered) = (unsafe { with_sheet(hwnd, |state| state.needle.clone()) }) else {
        return;
    };
    let Some(what) = dialog::prompt(hwnd, "Replace", "Replace what:", &offered) else {
        return;
    };
    if what.is_empty() {
        return;
    }
    let Some(with) = dialog::prompt(hwnd, "Replace", &format!("Replace “{what}” with:"), "")
    else {
        return;
    };
    // SAFETY: a fresh borrow, taken after both dialogs have closed.
    unsafe {
        with_sheet(hwnd, |state| {
            let said = match state.app.replace(
                &find::Search::new(what.as_str()),
                &with,
                RecalcMode::Document,
            ) {
                Ok(done) => match done.recalc.filter(|recalc| recalc.spoiled > 0) {
                    Some(recalc) => notice::recalc_skipped(recalc.spoiled),
                    None if done.cells == 0 && done.refused.is_empty() => notice::not_found(&what),
                    None => {
                        let first = done.refused.first().map(|(hit, _)| hit.address());
                        notice::replaced(
                            done.cells,
                            first.as_deref().map(|first| (first, done.refused.len())),
                        )
                    }
                },
                Err(error) => error.to_string(),
            };
            state.needle = what;
            state.say(Some(said));
        });
    }
    refresh(hwnd);
}

/// File ▸ Import CSV… — a delimited file read in **at the cursor**, one undo step: a dialog for
/// the file and nothing else, since the delimiter is read from the content and the selection says
/// where the fields land (`csv::Import::sniffed`); every other knob is `grind sheet import-csv`'s.
fn import_csv(hwnd: HWND, with_options: bool) {
    // The dialog runs a nested message loop, so nothing may be borrowed across it (decision 7).
    let Some(path) = dialog::open_csv_path(hwnd) else {
        return;
    };
    let text = match std::fs::read(&path)
        .map_err(|error| error.to_string())
        .and_then(|bytes| csv::decode(bytes).map_err(str::to_owned))
    {
        Ok(text) => text,
        Err(why) => {
            return dialog::error(
                hwnd,
                &format!("Could not read {}:\n\n{why}", path.display()),
            );
        }
    };
    // The options, in words, when they were asked for (`csv::Import::amended`) — after the file,
    // so the dialog that chose it is not left open under a prompt.
    let words = match with_options {
        true => match dialog::prompt(
            hwnd,
            "Import CSV",
            "Options — delimiter=semicolon locale=de-DE text formulas trim no-dates:",
            "",
        ) {
            Some(words) => words,
            None => return,
        },
        false => String::new(),
    };
    // SAFETY: one borrow, taken after the dialog has closed.
    unsafe {
        with_sheet(hwnd, |state| {
            let at = state.selection.active;
            let options = match csv::Import::sniffed(&text).amended(&words) {
                Ok(options) => options,
                Err(why) => return state.say(Some(why)),
            };
            match state
                .app
                .import_csv(state.sheet, at, &text, &options, RecalcMode::Document)
            {
                Ok(outcome) => {
                    let spoiled = outcome.recalc.map_or(0, |recalc| recalc.spoiled);
                    state.say(match spoiled {
                        0 => Some(notice::imported(outcome.cells, &a1::format(None, at))),
                        n => Some(notice::recalc_skipped(n)),
                    });
                }
                Err(error) => state.say(Some(error.to_string())),
            }
        });
    }
    refresh(hwnd);
}

/// File ▸ Export CSV… — the selection, or everything the sheet uses when one cell is selected.
/// **Writes a file and changes nothing**: no undo entry, no dirty flag; what comes out is what
/// each cell shows (`App::export_csv`).
fn export_csv(hwnd: HWND) {
    // SAFETY: one borrow, released before the dialog — which runs a nested message loop.
    let suggested = unsafe {
        with_sheet(hwnd, |state| {
            let stem = state
                .path
                .as_deref()
                .and_then(std::path::Path::file_stem)
                .map(|stem| stem.to_string_lossy().into_owned());
            format!("{}.csv", stem.unwrap_or_else(|| "Untitled".to_owned()))
        })
    }
    .unwrap_or_else(|| "Untitled.csv".to_owned());
    let Some(path) = dialog::save_csv_path(hwnd, &suggested) else {
        return;
    };
    let dialect = csv::Dialect::for_name(&path.file_name().unwrap_or_default().to_string_lossy());
    // SAFETY: one borrow, taken after the dialog has closed.
    let written = unsafe {
        with_sheet(hwnd, |state| {
            let (mut start, mut end) = state.selection.rect();
            if start == end {
                let (rows, cols) = state.app.used_extent(state.sheet).unwrap_or((0, 0));
                if rows == 0 || cols == 0 {
                    return Err("There is nothing in this sheet to export".to_owned());
                }
                (start, end) = (Pos::new(0, 0), Pos::new(rows - 1, cols - 1));
            }
            let options = csv::Export {
                dialect,
                ..csv::Export::default()
            };
            state
                .app
                .export_csv(state.sheet, start, end, &options)
                .map_err(|error| error.to_string())
                .map(|text| (text, start, end))
        })
    };
    let Some(written) = written else {
        return;
    };
    let (text, start, end) = match written {
        Ok(written) => written,
        Err(why) => return dialog::error(hwnd, &why),
    };
    if let Err(error) = grind_core::atomic::write(&path, text) {
        return dialog::error(
            hwnd,
            &format!("Could not write {}:\n\n{error}", path.display()),
        );
    }
    // SAFETY: one borrow, and nothing inside it dispatches.
    unsafe {
        with_sheet(hwnd, |state| {
            state.say(Some(notice::exported(
                &a1::format(None, start),
                &a1::format(None, end),
                &path.file_name().unwrap_or_default().to_string_lossy(),
            )));
        });
    }
    refresh(hwnd);
}

// --- sheets ---

/// The tab under a client-space point, if the grid's status bar has one there.
fn sheet_tab_at(state: &Sheet, x: f64, y: f64) -> Option<crate::sheet::tabs::Target> {
    crate::sheet::tabs::hit(&state.tabs.borrow(), x, y)
}

/// Whether a `WM_CONTEXTMENU` landed on a sheet tab — and if it did, bring that sheet to the
/// front first, so the menu's verbs act on the tab that was right-clicked. The message carries
/// *screen* coordinates, and `(-1, -1)` for the keyboard, which is never on a tab.
fn context_on_tab(hwnd: HWND, lparam: LPARAM) -> bool {
    if is_welcome(hwnd) || is_text(hwnd) {
        return false;
    }
    let (x, y) = point(lparam);
    if (x, y) == (-1.0, -1.0) {
        return false;
    }
    let mut at = POINT {
        x: x.round() as i32,
        y: y.round() as i32,
    };
    // SAFETY: `at` is a live local the call writes through.
    unsafe {
        let _ = ScreenToClient(hwnd, &mut at);
    }
    // SAFETY: one borrow, nothing inside dispatches.
    let tab = unsafe {
        with_sheet(hwnd, |state| {
            sheet_tab_at(state, f64::from(at.x), f64::from(at.y))
        })
    }
    .flatten();
    match tab {
        Some(crate::sheet::tabs::Target::Sheet(index)) => {
            sheet_show(hwnd, index);
            true
        }
        _ => false,
    }
}

/// Bring one sheet to the front — a tab clicked. Its own selection starts at `A1`, as
/// Ctrl+PageDown's does.
fn sheet_show(hwnd: HWND, index: usize) {
    commit_edit(hwnd, None);
    // SAFETY: one borrow.
    unsafe {
        with_sheet(hwnd, |state| {
            if index < state.app.sheet_count() && index != state.sheet {
                state.sheet = index;
                state.selection = Selection::default();
            }
        });
    }
    refresh(hwnd);
}

/// Move to another sheet, keeping the selection and the view — clamped, not wrapped:
/// Ctrl+PageDown on the last sheet does nothing, as in Excel.
fn sheet_step(hwnd: HWND, by: i64) {
    // SAFETY: one borrow.
    unsafe {
        with_sheet(hwnd, |state| {
            let last = state.app.sheet_count().saturating_sub(1) as i64;
            let at = (state.sheet as i64 + by).clamp(0, last.max(0));
            state.sheet = at as usize;
        });
    }
    refresh(hwnd);
}

fn sheet_add(hwnd: HWND) {
    // SAFETY: one borrow, released before the prompt — which runs a nested message loop.
    let Some(suggested) = (unsafe { with_sheet(hwnd, |state| state.app.fresh_sheet_name()) })
    else {
        return;
    };
    let Some(name) = dialog::prompt(hwnd, "Add Sheet", "Name for the new sheet:", &suggested)
    else {
        return;
    };
    // SAFETY: a fresh borrow, taken after the dialog rather than across it.
    let refused = unsafe {
        with_sheet(hwnd, |state| match state.app.add_sheet(&name) {
            Ok(at) => {
                state.sheet = at;
                state.selection = Selection::default();
                None
            }
            // The core's own sentence, not a second copy of the rule: an empty name and a
            // duplicate are both refused by `check_sheet_name`, which says why.
            Err(error) => Some(error.to_string()),
        })
    };
    if let Some(message) = refused.flatten() {
        dialog::error(hwnd, &message);
    }
    refresh(hwnd);
}

fn sheet_rename(hwnd: HWND) {
    // SAFETY: one borrow, released before the prompt.
    let Some((sheet, current)) = (unsafe {
        with_sheet(hwnd, |state| {
            (
                state.sheet,
                state.app.sheet_name(state.sheet).unwrap_or_default(),
            )
        })
    }) else {
        return;
    };
    let Some(name) = dialog::prompt(hwnd, "Rename Sheet", "New name:", &current) else {
        return;
    };
    // SAFETY: a fresh borrow, taken after the dialog rather than across it.
    let refused = unsafe {
        with_sheet(hwnd, |state| match state.app.rename_sheet(sheet, &name) {
            // The answer is **how many references were rewritten**: a rename that carried three
            // hundred formulas is something to be told about, and taken back in one step.
            Ok(0) => None,
            Ok(rewritten) => {
                state.say(Some(notice::references_renamed(rewritten)));
                None
            }
            Err(error) => Some(error.to_string()),
        })
    };
    if let Some(message) = refused.flatten() {
        dialog::error(hwnd, &message);
    }
    refresh(hwnd);
}

/// Delete the current sheet, after asking. The one destructive thing this milestone can do that
/// undo alone would not make obvious, which is why it is the one that confirms.
fn sheet_delete(hwnd: HWND) {
    // SAFETY: one borrow, released before the question.
    let Some((sheet, name)) = (unsafe {
        with_sheet(hwnd, |state| {
            (
                state.sheet,
                state.app.sheet_name(state.sheet).unwrap_or_default(),
            )
        })
    }) else {
        return;
    };
    // The last sheet is refused by `App::remove_sheet` itself, with a sentence saying why, and
    // that refusal arrives in the error box below. There is deliberately no check here: a rule
    // the core holds and a shell restates is a rule with two spellings.
    if !dialog::confirm(
        hwnd,
        &format!("Delete {name}?"),
        &format!("{name} and everything on it will be deleted."),
        "Delete",
    ) {
        return;
    }
    // SAFETY: a fresh borrow, taken after the question rather than across it.
    let refused = unsafe {
        with_sheet(hwnd, |state| match state.app.remove_sheet(sheet) {
            Ok(()) => {
                state.sheet = sheet.min(state.app.sheet_count().saturating_sub(1));
                state.selection = Selection::default();
                None
            }
            Err(error) => Some(error.to_string()),
        })
    };
    if let Some(message) = refused.flatten() {
        dialog::error(hwnd, &message);
    }
    refresh(hwnd);
}

// --- files ---

/// Save to the path the document came from, or ask for one. `true` means it is on disk.
fn save(hwnd: HWND) -> bool {
    // SAFETY: one borrow, released before any dialog.
    let path = unsafe { with_pane(hwnd, |pane| pane.path()) }.flatten();
    match path {
        Some(path) => write(hwnd, &path),
        None => save_as(hwnd),
    }
}

fn save_as(hwnd: HWND) -> bool {
    // SAFETY: one borrow, released before the dialog — which runs a nested message loop.
    let (suggested, kind) =
        unsafe { with_pane(hwnd, |pane| (pane.suggested(), pane.kind())) }.unwrap_or((None, None));
    // Nothing to save: the welcome screen holds no document, and `menu::on_welcome` keeps this verb
    // off its menu bar — so this is only reachable by a stale accelerator, and saying nothing is
    // the right answer to one.
    let Some(kind) = kind else {
        return false;
    };
    let Some(path) = dialog::save_path(hwnd, suggested.as_deref(), kind) else {
        return false;
    };
    write(hwnd, &path)
}

/// Write the document. A failure must never be quiet: the work is still only in memory, and a
/// close going ahead would lose it. Which form is written is `Form::from_path`'s answer to the
/// chosen extension, inside `grind_sheet::write_file`.
fn write(hwnd: HWND, path: &Path) -> bool {
    // SAFETY: one borrow. `save_file` only reads the document: it notifies nothing, opens nothing,
    // dispatches nothing. No "Saved" banner: the title's `*` clearing is the confirmation.
    let failed = unsafe { with_pane(hwnd, |pane| pane.save_file(path).err()) }.flatten();
    match failed {
        Some(message) => {
            dialog::error(
                hwnd,
                &format!("Could not save {}:\n\n{message}", path.display()),
            );
            false
        }
        None => {
            add_recent(path);
            refresh(hwnd);
            true
        }
    }
}

/// Replace what the window shows — **including with a document of the other kind**. The observer
/// is registered again, being bound to an `App`, not the window; registering after the file is
/// read is why no "a load is not an edit" flag is needed.
fn adopt(hwnd: HWND, pane: Pane) {
    // SAFETY: one borrow. `set_observer` stores an `Arc` and dispatches nothing, and dropping the
    // pane that was there releases its fonts and its document with no message in between.
    unsafe {
        with_pane(hwnd, |slot| {
            let observer = Arc::new(Changed(hwnd.0 as isize));
            match &pane {
                Pane::Sheet(sheet) => sheet.app.set_observer(observer),
                Pane::Text(text) => text.app.set_observer(observer),
                // No document, so nothing to observe — and nothing that could mark a title.
                Pane::Welcome(_) => {}
            }
            *slot = pane;
        });
    }
    // A window that has just become a spreadsheet needs the two `EDIT`s a spreadsheet edits
    // through; one that has just become a document or the welcome screen needs nothing and gets
    // nothing.
    make_children(hwnd);
    // The menu is built per surface (`build_menu`), and File ▸ Open, either New or the start
    // screen can change that surface in the same window — so it is rebuilt here rather than only
    // once at `WM_CREATE`.
    build_menu(hwnd);
    refresh(hwnd);
}

/// A new, empty document of the kind the caller names — either kind, from either pane.
fn new_document(hwnd: HWND, kind: DocumentKind) {
    if !offer_to_save(hwnd) {
        return;
    }
    // SAFETY: one borrow, for the theme the new pane starts in.
    let theme = unsafe { with_pane(hwnd, |pane| pane.theme()) }.unwrap_or_else(theme::current);
    if let Ok(pane) = opened(Some(kind), None, theme) {
        adopt(hwnd, pane);
    }
}

/// Back to the welcome screen, closing what the window showed: ask about unsaved work, then adopt
/// a new pane — the operation above with `None` for the kind.
fn welcome_screen(hwnd: HWND) {
    if !offer_to_save(hwnd) {
        return;
    }
    // SAFETY: one borrow, for the theme the new pane starts in.
    let theme = unsafe { with_pane(hwnd, |pane| pane.theme()) }.unwrap_or_else(theme::current);
    if let Ok(pane) = opened(None, None, theme) {
        adopt(hwnd, pane);
    }
}

/// The welcome screen's verbs: everything needing a document is a no-op, the match exhaustive for
/// `text_command`'s reason. The ones that mean something are `menu::on_welcome`'s list, and the
/// cards name commands that `welcome.rs` checks are in a menu.
fn welcome_command(hwnd: HWND, command: Command) {
    match command {
        Command::NewSheet => new_document(hwnd, DocumentKind::Spreadsheet),
        Command::NewText => new_document(hwnd, DocumentKind::Text),
        Command::Open => open_document(hwnd),
        Command::Exit => {
            // SAFETY: nothing is borrowed, and posting only queues the message.
            unsafe {
                let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
            }
        }
        Command::Shortcuts => show_shortcuts(hwnd),
        Command::About => dialog::about(hwnd),
        Command::Licences => dialog::licences(hwnd),
        // Everything else acts on a document, and there is none. `menu::items_for` keeps all of
        // them off this pane's menu bar; these arms are what makes a stale accelerator harmless.
        Command::Welcome
        | Command::Save
        | Command::SaveAs
        | Command::Undo
        | Command::Redo
        | Command::Cut
        | Command::Copy
        | Command::Paste
        | Command::ClearCells
        | Command::FillDown
        | Command::FillRight
        | Command::CopyValue
        | Command::FormulaToValue
        | Command::FillAcross
        | Command::MergeCells
        | Command::UnmergeCells
        | Command::InsertChart
        | Command::InsertBarChart
        | Command::InsertLineChart
        | Command::InsertPieChart
        | Command::ChartToBar
        | Command::ChartToLine
        | Command::ChartToPie
        | Command::PreviewChart
        | Command::DeleteChart
        | Command::RestyleChart
        | Command::MoveChart
        | Command::WrapText
        | Command::BordersAll
        | Command::BordersNone
        | Command::HideRows
        | Command::ShowRows
        | Command::HideColumns
        | Command::ShowColumns
        | Command::RowHeight
        | Command::FitColumns
        | Command::FitRows
        | Command::FitSheet
        | Command::ColumnWidth
        | Command::DefineName
        | Command::RenameName
        | Command::InlineName
        | Command::DeleteName
        | Command::DocumentLocale
        | Command::FindCalculation
        | Command::Evaluate
        | Command::GoTo
        | Command::Find
        | Command::FindNext
        | Command::FindPrevious
        | Command::Replace
        | Command::ImportCsv
        | Command::ImportCsvWith
        | Command::ExportCsv
        | Command::Recalculate
        | Command::FunctionList
        | Command::ExplainFormula
        | Command::ToggleFriendly
        | Command::ToggleFilter
        | Command::FormatTable
        | Command::FormatTableTotals
        | Command::AddRule
        | Command::RemoveRule
        | Command::CurrencyEuro
        | Command::CurrencyDollar
        | Command::CurrencyPound
        | Command::GroupThousands
        | Command::AlignLeft
        | Command::AlignCenter
        | Command::AlignRight
        | Command::IndentLess
        | Command::IndentMore
        | Command::TextRotation
        | Command::PickBackground
        | Command::NumberFormat
        | Command::FewerDecimals
        | Command::MoreDecimals
        | Command::SheetAdd
        | Command::SheetRename
        | Command::SheetDelete
        | Command::SheetNext
        | Command::SheetPrevious
        | Command::Bold
        | Command::Italic
        | Command::Underline
        | Command::Strike
        | Command::Code
        | Command::PickFamily
        | Command::PickSize
        | Command::PickColor
        | Command::PickHighlight
        | Command::ClearFormatting
        | Command::Title
        | Command::Subtitle
        | Command::Paragraph
        | Command::Heading1
        | Command::Heading2
        | Command::Heading3
        | Command::Outline
        | Command::BlockKindDialog
        | Command::InsertPicture
        | Command::AltText
        | Command::InsertTable
        | Command::Bookmark
        | Command::ParagraphStyle
        | Command::ParagraphUp
        | Command::ParagraphDown
        | Command::ParagraphDelete
        | Command::ImportMarkdown
        | Command::ExportMarkdown
        | Command::ExportPdf
        | Command::PrintPreview
        | Command::NextMisspelling
        | Command::SpellingLanguage
        | Command::PersonalDictionary
        | Command::ToggleSpelling
        | Command::Print
        | Command::ShowSource
        | Command::CheckDocument
        | Command::ToggleFormulas
        | Command::ZoomIn
        | Command::ZoomOut
        | Command::ZoomReset
        | Command::ToggleRoles
        | Command::ToggleNames => {}
    }
}

fn open_document(hwnd: HWND) {
    if !offer_to_save(hwnd) {
        return;
    }
    let Some(path) = dialog::open_path(hwnd) else {
        return;
    };
    // The kind is read from the bytes, before parsing (`grind_core::kind`) — so a `.fodt` chosen
    // here replaces the grid with a text pane in the same window.
    let kind = match crate::sniff(&path) {
        Ok(kind) => kind,
        Err(message) => {
            dialog::error(hwnd, &message);
            return;
        }
    };
    // SAFETY: one borrow, released before the document is read.
    let theme = unsafe { with_pane(hwnd, |pane| pane.theme()) }.unwrap_or_else(theme::current);
    // Read into a *new* pane rather than over the live one, so that a file that turns out to be
    // unreadable leaves the window showing what it was showing.
    match opened(Some(kind), Some(path.clone()), theme) {
        Ok(pane) => {
            adopt(hwnd, pane);
            add_recent(&path);
        }
        Err(error) => dialog::error(
            hwnd,
            &format!("Could not open {}:\n\n{error}", path.display()),
        ),
    }
}

/// Tell Windows this document was used, for the taskbar jump list and every file dialog's *Recent*
/// — the platform's own list, so this window keeps none. Best-effort.
fn add_recent(path: &Path) {
    use windows::Win32::UI::Shell::{SHARD_PATHW, SHAddToRecentDocs};
    let wide = gdi::wide(&path.to_string_lossy());
    // SAFETY: the buffer is a NUL-terminated local that outlives the call, which only reads it.
    unsafe {
        SHAddToRecentDocs(SHARD_PATHW.0 as u32, Some(wide.as_ptr().cast()));
    }
}

fn offer_to_save(hwnd: HWND) -> bool {
    // SAFETY: one borrow, released before the dialog — decision 7's rule, and the reason this is
    // a function rather than three lines in each of its three callers.
    let Some((dirty, name)) =
        (unsafe { with_pane(hwnd, |pane| (pane.dirty(), pane.document_name())) })
    else {
        return true;
    };
    if !dirty {
        return true;
    }
    match dialog::confirm_close(hwnd, &name) {
        Answer::Save => save(hwnd),
        Answer::Discard => true,
        Answer::Cancel => false,
    }
}

/// The close question, and the only place this window is destroyed on purpose.
fn close(hwnd: HWND) {
    commit_edit(hwnd, None);
    if !offer_to_save(hwnd) {
        return;
    }
    // SAFETY: nothing is borrowed. `DestroyWindow` sends `WM_DESTROY` and `WM_NCDESTROY`
    // synchronously, and the second of those is what frees the state.
    unsafe {
        let _ = DestroyWindow(hwnd);
    }
}

/// Everything one frame needs, drawn onto a device context — an `HDC` and the state, nothing about
/// the window, so [`render`] is a second caller.
fn draw_frame(dc: HDC, state: &Sheet) {
    let rows = state.geom.visible_rows();
    let cols = state.geom.visible_cols();
    let viewport = state
        .app
        .get_viewport_with(state.sheet, rows, cols, state.overlays)
        .unwrap_or_else(|_| {
            state
                .app
                .get_viewport(0, 0..0, 0..0)
                .expect("an empty rectangle of the first sheet always reads")
        });
    let status_right = status::status_right(&state.app, state.sheet, state.selection);
    let sheets: Vec<String> = (0..state.app.sheet_count())
        .map(|index| state.app.sheet_name(index).unwrap_or_default())
        .collect();
    let name = status::name_box_text(&state.app, state.sheet, state.selection);
    // While an in-cell edit is open the bar mirrors the control, which is what makes the strip a
    // read-out of *the cell* rather than of the document underneath it. When the control is on
    // the bar it is covering this text, and reading it back would be drawing under a window.
    let editing = state.mode.is_editing();
    let formula = match editing && !state.editor_on_bar {
        true => window_text(state.editor),
        false => status::formula_bar_text(&state.app, state.sheet, state.selection),
    };
    // The friendly reading, if it is on and there is a formula to read — never while editing,
    // because the text under the caret has to be the text that will be stored. A formula this
    // build cannot parse has no reading, and then the bar shows what it always shows.
    let friendly = (state.friendly && !editing)
        .then(|| assist::friendly_line(&formula))
        .flatten();
    // With the Names overlay on, a formula that uses a defined name reads through it
    // (`=tax_rate*subtotal`) in the bar's reading slot where the friendly one did not claim it —
    // the GNOME window's order, friendly first. Never while editing, for the reason above.
    let friendly = friendly.or_else(|| {
        (state.overlays.names && !editing)
            .then(|| {
                grind_sheet::place::named_reading(
                    &state.app,
                    state.sheet,
                    state.selection.active,
                    &formula,
                )
            })
            .flatten()
    });
    // The format strip reads the **active** cell, as every spreadsheet's toolbar does
    // (`sheet/format.rs`).
    let active = state.selection.active;
    let style = state
        .app
        .style_at(state.sheet, active)
        .ok()
        .flatten()
        .unwrap_or_default();
    let number = state.app.format_at(state.sheet, active).ok().flatten();
    // With formulas shown, the formula's own text stands in for the result in every formula cell
    // the viewport covers (`App::input_text`, display syntax).
    let mut formula_text = std::collections::HashMap::new();
    if state.formulas {
        for row in state.geom.visible_rows() {
            for col in state.geom.visible_cols() {
                let pos = Pos::new(row, col);
                if state.app.formula(state.sheet, pos).ok().flatten().is_some()
                    && let Ok(text) = state.app.input_text(state.sheet, pos)
                {
                    formula_text.insert((row, col), text);
                }
            }
        }
    }
    let charts = state.app.charts(state.sheet).unwrap_or_default();
    let chart_data: Vec<Option<grind_sheet::ChartData>> = (0..charts.len())
        .map(|index| state.app.chart_data(state.sheet, index).ok())
        .collect();
    let tabs = draw::paint(
        dc,
        &Frame {
            charts: &charts,
            chart_data: &chart_data,
            chart_selected: state.chart_now(),
            chart_grab: state.chart_grab,
            formula_text: &formula_text,
            geom: &state.geom,
            theme: state.theme,
            format: FormatStrip {
                style: &style,
                number: &format::face(number.as_ref()),
                number_set: number.is_some(),
                hover: state.format_hover,
                pressed: state.format_pressed,
            },
            viewport: &viewport,
            sheets: &sheets,
            sheet: state.sheet,
            status_right: &status_right,
            status_picker: state.spelling.chip(false),
            name: &name,
            formula: friendly.as_deref().unwrap_or(&formula),
            friendly: friendly.is_some(),
            banner: state.banner.as_deref(),
            hint: &state.hint,
            selection: state.selection,
            filter: state.app.filter(state.sheet).ok().flatten(),
            used: state.app.used_extent(state.sheet).unwrap_or((0, 0)),
            font_px: (scale(theme::text::CELL, state.geom.dpi) * state.geom.zoom).round() as i32,
            caption_px: scale(theme::text::CAPTION, state.geom.dpi).round() as i32,
            body_px: scale(theme::text::BODY, state.geom.dpi).round() as i32,
            face: face(),
        },
    );
    *state.tabs.borrow_mut() = tabs;
}

/// One frame, through the back buffer.
fn paint(hwnd: HWND) {
    // The text pane's caret is Windows' own (`place_system_caret`), and `BitBlt` paints over it,
    // leaving a stray rectangle unless it is hidden first and shown after. The grid has none.
    let has_caret = is_text(hwnd);
    if has_caret {
        // SAFETY: no arguments to outlive the call.
        unsafe {
            let _ = HideCaret(Some(hwnd));
        }
    }
    let mut ps = PAINTSTRUCT::default();
    // SAFETY: `BeginPaint`/`EndPaint` are paired on every path below, including the early
    // return when the back buffer cannot be made.
    unsafe {
        let dc: HDC = BeginPaint(hwnd, &mut ps);
        let rect = gdi::client_rect(hwnd);
        if let Some(buffer) = BackBuffer::new(dc, rect.right - rect.left, rect.bottom - rect.top) {
            with_pane(hwnd, |pane| {
                buffer.clear(pane.theme().background);
                match pane {
                    Pane::Sheet(sheet) => draw_frame(buffer.dc(), sheet),
                    Pane::Text(text) => draw_text_frame(buffer.dc(), text, true),
                    Pane::Welcome(state) => draw_welcome_frame(buffer.dc(), state),
                }
            });
            buffer.present(dc);
        }
        let _ = EndPaint(hwnd, &ps);
    }
    if has_caret {
        // Repositioned after every repaint, not trusted to whichever handler invalidated the
        // window — a resize or a theme change included — so a caret moved in a dozen places is
        // placed in one.
        place_system_caret(hwnd);
    }
}

// --- the welcome screen ---
//
// The third pane, and the smallest — no document, viewport, history, scrolling or save — with the
// four things the others have: a refresh, a frame taking an `HDC`, a hit test and a keymap.

/// Whether this window is showing the welcome screen rather than a document.
fn is_welcome(hwnd: HWND) -> bool {
    // SAFETY: one borrow, for one bit; nothing inside dispatches.
    unsafe { with_pane(hwnd, |pane| matches!(pane, Pane::Welcome(_))) }.unwrap_or(false)
}

/// Run `f` with the welcome screen's state, or `None` when the window is showing a document.
unsafe fn with_welcome<T>(hwnd: HWND, f: impl FnOnce(&mut Welcome) -> T) -> Option<T> {
    // SAFETY: the caller's, unchanged — see [`with_pane`].
    unsafe { with_pane(hwnd, |pane| pane.welcome_mut().map(f)) }.flatten()
}

/// The welcome screen's [`refresh`]: re-measure, retitle, repaint. The scrollbars are flattened,
/// since a window that was a spreadsheet still has its thumbs.
fn welcome_refresh(hwnd: HWND) {
    let rect = gdi::client_rect(hwnd);
    // SAFETY: `hwnd` is this window's.
    let dpi = unsafe { GetDpiForWindow(hwnd) }.max(96);
    // SAFETY: one borrow; `relayout` is arithmetic and dispatches nothing.
    unsafe {
        with_welcome(hwnd, |state| {
            state.relayout(
                f64::from(rect.right - rect.left),
                f64::from(rect.bottom - rect.top),
                dpi,
            );
        });
    }
    let flat = SCROLLINFO {
        cbSize: u32::try_from(std::mem::size_of::<SCROLLINFO>()).expect("small"),
        fMask: SCROLLINFO_MASK(SIF_RANGE.0 | SIF_PAGE.0 | SIF_POS.0),
        nMin: 0,
        nMax: 0,
        nPage: 1,
        nPos: 0,
        nTrackPos: 0,
    };
    // SAFETY: the borrow above is released; `info` is a live local read for the length of each
    // call, and `SetWindowTextW` dispatches, which is why it is out here.
    let title = unsafe { with_pane(hwnd, |pane| pane.title()) };
    unsafe {
        SetScrollInfo(hwnd, SB_VERT, &flat, true);
        SetScrollInfo(hwnd, SB_HORZ, &flat, true);
        if let Some(title) = title
            && window_text(hwnd) != title
        {
            let wide = gdi::wide(&title);
            let _ = SetWindowTextW(hwnd, PCWSTR(wide.as_ptr()));
        }
        let _ = InvalidateRect(Some(hwnd), None, false);
    }
}

/// One frame of the welcome screen. Takes an `HDC` and the state and nothing about the window,
/// which is what makes [`render`] a second caller rather than a second drawing path.
fn draw_welcome_frame(dc: HDC, state: &Welcome) {
    welcome::paint(
        dc,
        &state.page,
        &welcome::Frame {
            theme: state.theme,
            face: face(),
            hover: state.hover,
            pressed: state.pressed,
            focus: state.focus,
        },
    );
}

/// The pointer moved over the welcome screen: which card it is on, repainted only when that changed.
fn welcome_mouse_move(hwnd: HWND, x: f64, y: f64) {
    // SAFETY: one borrow; nothing inside dispatches.
    let moved = unsafe {
        with_welcome(hwnd, |state| {
            let hover = state.page.hit(x, y);
            let moved = hover != state.hover;
            state.hover = hover;
            moved
        })
    };
    if moved == Some(true) {
        // SAFETY: the borrow above is released.
        unsafe {
            let _ = InvalidateRect(Some(hwnd), None, false);
        }
    }
}

/// A press on a card. It is *held*, not run — [`welcome_button_up`] acts on release, so moving
/// off the card before letting go takes it back, which is what every button on this platform
/// does and what the text pane's format strip already does here.
fn welcome_button_down(hwnd: HWND, x: f64, y: f64) {
    // SAFETY: one borrow; nothing inside dispatches.
    unsafe {
        with_welcome(hwnd, |state| {
            let hit = state.page.hit(x, y);
            state.hover = hit;
            state.pressed = hit;
            // A click also moves the keyboard's focus onto the card, so Enter afterwards repeats
            // what the pointer just did rather than whatever was focused before.
            if let Some(at) = hit {
                state.focus = at;
            }
        });
    }
    // SAFETY: the borrow is released. Both of these dispatch.
    unsafe {
        let _ = SetCapture(hwnd);
        let _ = SetFocus(Some(hwnd));
        let _ = InvalidateRect(Some(hwnd), None, false);
    }
}

fn welcome_button_up(hwnd: HWND) {
    // SAFETY: one borrow, released before the verb runs — two of the three open a nested message
    // loop (a file dialog, a close question), which decision 7 forbids holding a borrow across.
    let pressed = unsafe {
        with_welcome(hwnd, |state| {
            state.pressed.take().filter(|at| state.hover == Some(*at))
        })
    }
    .flatten();
    // SAFETY: the borrow above is released.
    unsafe {
        let _ = ReleaseCapture();
        let _ = InvalidateRect(Some(hwnd), None, false);
    }
    if let Some(at) = pressed {
        do_command(hwnd, welcome::Choice::ALL[at].command());
    }
}

/// The welcome screen's keyboard: step through the cards and run one. Tab is Down and Shift+Tab
/// Up, the convention for buttons with no dialog manager; `false` hands the key to
/// `DefWindowProcW`, so Alt+F4 and the menu bar's Alt work.
fn welcome_key(hwnd: HWND, vk: u32) -> bool {
    use crate::sheet::keymap::Key;

    let mods = mods();
    if let Some(command) = menu::accelerator(keymap::key_for(vk), mods) {
        do_command(hwnd, command);
        return true;
    }
    let step = match keymap::key_for(vk) {
        Key::Down => 1,
        Key::Up => -1,
        Key::Tab if mods.shift => -1,
        Key::Tab => 1,
        Key::Return => {
            // SAFETY: one borrow, released before the verb — which may open a file dialog.
            let focus = unsafe { with_welcome(hwnd, |state| state.focus) };
            if let Some(at) = focus {
                do_command(hwnd, welcome::Choice::ALL[at].command());
            }
            return true;
        }
        _ => return false,
    };
    // SAFETY: one borrow; nothing inside dispatches.
    unsafe {
        with_welcome(hwnd, |state| {
            state.focus = welcome::step(state.focus, step);
            // The keyboard owns the highlight while it is being used: a hover left over from
            // wherever the pointer happens to be resting would draw a second card as lit.
            state.hover = None;
        });
        let _ = InvalidateRect(Some(hwnd), None, false);
    }
    true
}

// --- the text pane ---
//
// The word processor's pane, in the grid's shape: a refresh rebuilding everything derived, a frame
// taking an `HDC` and no `HWND`, and handlers borrowing the state for one message — two documents
// in one window, not two programs in one binary.

/// Whether this window shows a document rather than a spreadsheet — asked at the top of each
/// shared handler, since most messages are one pane's.
fn is_text(hwnd: HWND) -> bool {
    // SAFETY: one borrow, for one bit; nothing inside dispatches.
    unsafe { with_pane(hwnd, |pane| matches!(pane, Pane::Text(_))) }.unwrap_or(false)
}

/// Show the system caret at [`Text::caret_geometry`], creating it first: `CreateCaret` again
/// rather than only `SetCaretPos`, since a heading's caret is taller and recreating sets either
/// size. Windows blinks it at the user's `GetCaretBlinkTime`, which is why a system caret is worth
/// having; recreating resets the blink phase each keystroke, as every editor's caret does.
fn place_system_caret(hwnd: HWND) {
    // SAFETY: one borrow; nothing inside dispatches.
    let geometry = unsafe { with_text(hwnd, |text| text.caret_geometry()) }.flatten();
    let Some((x, y, width, height)) = geometry else {
        return;
    };
    // SAFETY: `CreateCaret`, `SetCaretPos` and `ShowCaret` are user32's caret API — one per
    // thread, paired with `DestroyCaret` in `WM_KILLFOCUS` — and none of them runs a nested
    // message loop.
    unsafe {
        let _ = CreateCaret(hwnd, None, width, height);
        let _ = SetCaretPos(x, y);
        let _ = ShowCaret(Some(hwnd));
    }
}

/// The text pane's [`refresh`]: rebuild the bands, re-measure the document, follow the caret.
fn text_refresh(hwnd: HWND) {
    let rect = gdi::client_rect(hwnd);
    // SAFETY: `hwnd` is this window's.
    let dpi = unsafe { GetDpiForWindow(hwnd) }.max(96);
    // SAFETY: one borrow; `relayout` measures text and dispatches nothing.
    unsafe {
        with_text(hwnd, |text| {
            text.reconsider_spelling();
            text.relayout(
                f64::from(rect.right - rect.left),
                f64::from(rect.bottom - rect.top),
                dpi,
            );
            text.reveal();
            text_scrollbars(hwnd, text);
        });
    }
    // SAFETY: the borrow above is released before the caption is rewritten, which dispatches.
    let title = unsafe { with_pane(hwnd, |pane| pane.title()) };
    unsafe {
        if let Some(title) = title
            && window_text(hwnd) != title
        {
            let wide = gdi::wide(&title);
            let _ = SetWindowTextW(hwnd, PCWSTR(wide.as_ptr()));
        }
        let _ = InvalidateRect(Some(hwnd), None, false);
    }
    // Called from every motion and edit: `CreateCaret` with no change is documented as harmless,
    // and cheap next to a repaint.
    place_system_caret(hwnd);
}

/// The scrollbars for a document, in **pixels** where the grid's are tracks — a document is as
/// tall as it measures, and a line is no unit anything else here counts in. The horizontal bar is
/// disabled: the measure is fixed.
fn text_scrollbars(hwnd: HWND, text: &Text) {
    let page = text.page.body().h.max(1.0);
    let info = SCROLLINFO {
        cbSize: u32::try_from(std::mem::size_of::<SCROLLINFO>()).expect("small"),
        fMask: SCROLLINFO_MASK(SIF_RANGE.0 | SIF_PAGE.0 | SIF_POS.0),
        nMin: 0,
        nMax: text.flow.height().round() as i32,
        nPage: page.round() as u32,
        nPos: text.page.scroll.round() as i32,
        nTrackPos: 0,
    };
    // SAFETY: `info` is a live, fully initialised local read for the length of the call.
    unsafe {
        SetScrollInfo(hwnd, SB_VERT, &info, true);
        let flat = SCROLLINFO {
            nMax: 0,
            nPage: 1,
            nPos: 0,
            ..info
        };
        SetScrollInfo(hwnd, SB_HORZ, &flat, true);
    }
}

/// Move the view by a number of pixels, clamped to the document.
fn text_scroll_by(hwnd: HWND, delta: f64) {
    // SAFETY: one borrow; nothing inside dispatches.
    unsafe {
        with_text(hwnd, |text| {
            let page = text.page.body().h;
            let limit = text.flow.limit(page);
            let to = (text.page.scroll + delta).clamp(0.0, limit);
            if to == text.page.scroll {
                return;
            }
            text.page.scroll = to;
            text_scrollbars(hwnd, text);
        });
        let _ = InvalidateRect(Some(hwnd), None, false);
    }
}

/// One scrollbar message on the document.
fn text_scroll(hwnd: HWND, wparam: WPARAM, vertical: bool) {
    if !vertical {
        return;
    }
    let code = (wparam.0 & 0xffff) as i32;
    let thumb = ((wparam.0 >> 16) & 0xffff) as i32;
    // SAFETY: one borrow, for two numbers; nothing inside dispatches.
    let Some((at, page, line)) = (unsafe {
        with_text(hwnd, |text| {
            (
                text.page.scroll,
                text.page.body().h,
                text.fonts.as_ref().map(Fonts::body_px).unwrap_or(15.0) * 1.3,
            )
        })
    }) else {
        return;
    };
    let delta = match scroll_code(code) {
        SB_LINEUP => -line,
        SB_LINEDOWN => line,
        SB_PAGEUP => -page,
        SB_PAGEDOWN => page,
        SB_TOP => -at,
        SB_BOTTOM => f64::MAX / 4.0,
        // Dragging reports an absolute position rather than a delta, and both codes are handled
        // so the view follows the thumb live instead of jumping on release.
        SB_THUMBPOSITION | SB_THUMBTRACK => f64::from(thumb) - at,
        _ => return,
    };
    text_scroll_by(hwnd, delta);
}

/// The wheel over a document: the user's own `SPI_GETWHEELSCROLLLINES`, in lines of prose.
fn text_wheel(hwnd: HWND, notches: f64, lines: u32) {
    // SAFETY: one borrow, for one number; nothing inside dispatches.
    let Some((line, page)) = (unsafe {
        with_text(hwnd, |text| {
            (
                text.fonts.as_ref().map(Fonts::body_px).unwrap_or(15.0) * 1.3,
                text.page.body().h,
            )
        })
    }) else {
        return;
    };
    // Zero means "do not scroll", which is a real setting; `WHEEL_PAGESCROLL` means a screenful.
    let step = match lines {
        0 => return,
        u32::MAX => page,
        n => f64::from(n) * line,
    };
    text_scroll_by(hwnd, -notches * step);
}

/// One keystroke in the text pane. `false` hands it back to `DefWindowProc`.
fn text_key(hwnd: HWND, vk: u32) -> bool {
    let key = keymap::key_for(vk);
    let mods = mods();
    // Verbs first, so that Ctrl+S is Save rather than whatever `S` would otherwise be — the same
    // order `sheet/state.rs` uses, and the same table.
    if let Some(command) = menu::accelerator(key, mods) {
        do_command(hwnd, command);
        return true;
    }
    // F5 and Ctrl+G are the grid's own way into `Command::GoTo` (`sheet/keymap.rs`'s `Action::GoTo`)
    // rather than the accelerator table's, so this pane matches them the same way instead of
    // teaching `menu::accelerator` a second spelling for one command.
    if (key == keymap::Key::F5 && !mods.ctrl) || (key == keymap::Key::Char('G') && mods.ctrl) {
        text_go_to(hwnd);
        return true;
    }
    // SAFETY: one borrow, for the page size; nothing inside dispatches.
    let page = unsafe { with_text(hwnd, |text| text.page_lines()) }.unwrap_or(20);
    let Some(action) = text::keymap::action_for(key, mods, page) else {
        return false;
    };
    match action {
        text::keymap::Action::Move { motion, extend } => {
            // SAFETY: one borrow; every answer inside is the core's, and none of them dispatches.
            unsafe {
                with_text(hwnd, |text| {
                    text.navigate(motion, extend);
                    // A moving caret is a visible caret: blinking it off under the user's finger
                    // is the one thing a caret must never do.
                    text.caret_on = true;
                    text_scrollbars(hwnd, text);
                });
                let _ = InvalidateRect(Some(hwnd), None, false);
            }
        }
        text::keymap::Action::SelectAll => {
            // SAFETY: one borrow; nothing inside dispatches.
            unsafe {
                with_text(hwnd, |text| {
                    text.anchor = START;
                    text.resume = None;
                    text.caret = text.last_caret();
                    text.goal_x = None;
                    text.reveal();
                });
                let _ = InvalidateRect(Some(hwnd), None, false);
            }
        }
        text::keymap::Action::Erase { forward } => text_erase(hwnd, forward),
        text::keymap::Action::Split => text_split(hwnd),
        text::keymap::Action::Tab { back } => text_indent(hwnd, back),
    }
    true
}

/// A character, after the layout, the IME and any surrogate pair (`typed_char`). Escape (`\u{1b}`)
/// and Enter (`\r`) arrive too, since `TranslateMessage` makes a `WM_CHAR` for both; Return and
/// Backspace are handled as keys.
fn text_char(hwnd: HWND, c: char) -> bool {
    let m = mods();
    if m.ctrl || m.alt || c.is_control() {
        return false;
    }
    text_type(hwnd, c);
    true
}

/// Type one character through `App::type_markdown`, so `**bold**` is read as it is typed. A
/// selection is dropped first, as [`text_insert`] drops one, since `type_markdown` only inserts.
fn text_type(hwnd: HWND, c: char) {
    // SAFETY: one borrow. `type_markdown` notifies, and the observer posts rather than sends.
    unsafe {
        with_text(hwnd, |text| {
            text_drop_selection(text);
            let at = text.caret;
            match text
                .app
                .type_markdown(at, &c.to_string(), text.resume.as_ref())
            {
                Ok(typed) => {
                    text.place(typed.caret, false);
                    text.resume = typed.resume;
                    text.caret_on = true;
                    text.say(None);
                }
                Err(error) => text.say(Some(error.to_string())),
            }
        });
    }
    refresh(hwnd);
}

/// Type text at the caret, replacing the selection. Erase and insert are two undo steps — `App`
/// has no "replace", and one would be a core change.
fn text_insert(hwnd: HWND, what: &str) {
    // SAFETY: one borrow. `insert_text` notifies, and the observer posts rather than sends.
    unsafe {
        with_text(hwnd, |text| {
            text_drop_selection(text);
            let at = text.caret;
            match text.app.insert_text(at, what) {
                Ok(()) => {
                    text.place(
                        Caret {
                            block: at.block,
                            offset: at.offset + what.chars().count(),
                        },
                        false,
                    );
                    text.caret_on = true;
                    text.say(None);
                }
                Err(error) => text.say(Some(error.to_string())),
            }
        });
    }
    refresh(hwnd);
}

/// Backspace and Delete.
fn text_erase(hwnd: HWND, forward: bool) {
    // SAFETY: one borrow. `erase` notifies, and the observer posts rather than sends.
    unsafe {
        with_text(hwnd, |text| {
            if text_drop_selection(text) {
                text.caret_on = true;
                return;
            }
            let at = text.caret;
            // At a block's edge the character to remove is the *break* between two blocks, which
            // is `erase` across the boundary rather than a join: one verb, one undo step, and no
            // second rule about what happens to the kinds.
            let (from, to) = match forward {
                true if at.offset < text.block_len(at.block) => (
                    at,
                    Caret {
                        block: at.block,
                        offset: at.offset + 1,
                    },
                ),
                true if at.block + 1 < text.app.block_count() => (
                    at,
                    Caret {
                        block: at.block + 1,
                        offset: 0,
                    },
                ),
                false if at.offset > 0 => (
                    Caret {
                        block: at.block,
                        offset: at.offset - 1,
                    },
                    at,
                ),
                false if at.block > 0 => (
                    Caret {
                        block: at.block - 1,
                        offset: text.block_len(at.block - 1),
                    },
                    at,
                ),
                // The very start or the very end of the document: nothing to erase, and nothing
                // to say about it either.
                _ => return,
            };
            match text.app.erase(from, to) {
                Ok(_) => {
                    text.place(from, false);
                    text.caret_on = true;
                    text.say(None);
                }
                Err(error) => text.say(Some(error.to_string())),
            }
        });
    }
    refresh(hwnd);
}

/// The Return key: one block becomes two, and the caret goes to the front of the second.
fn text_split(hwnd: HWND) {
    // SAFETY: one borrow. `split_block` notifies, and the observer posts rather than sends.
    unsafe {
        with_text(hwnd, |text| {
            text_drop_selection(text);
            let at = text.caret;
            match text.app.split_block(at) {
                Ok(()) => {
                    text.place(
                        Caret {
                            block: at.block + 1,
                            offset: 0,
                        },
                        false,
                    );
                    text.resume = None;
                    text.caret_on = true;
                    text.say(None);
                }
                Err(error) => text.say(Some(error.to_string())),
            }
        });
    }
    refresh(hwnd);
}

/// Erase whatever is selected, leaving the caret where it was; `true` when there was something.
/// Every edit begins with it: typing over a selection replaces it.
fn text_drop_selection(text: &mut Text) -> bool {
    if !text.has_selection() {
        return false;
    }
    let (from, to) = text.range();
    match text.app.erase(from, to) {
        Ok(_) => {
            text.place(from, false);
            text.say(None);
            true
        }
        Err(error) => {
            text.say(Some(error.to_string()));
            false
        }
    }
}

/// A press of the left button: the format strip first, since it sits over the document and a click
/// on chrome must not move the caret underneath; anywhere else the caret goes to the pointer, and
/// dragging extends.
fn text_button_down(hwnd: HWND, lparam: LPARAM) {
    let (x, y) = point(lparam);
    // The status bar's spelling language: Spelling Language…'s own chooser.
    // SAFETY: one borrow; arithmetic.
    if unsafe { with_text(hwnd, |text| text.page.status_picker().contains(x, y)) } == Some(true) {
        text_spelling_language(hwnd);
        return;
    }
    // SAFETY: one borrow; the hit test is arithmetic over the page geometry and dispatches
    // nothing.
    let hit = unsafe { with_text(hwnd, |text| text.page.strip_hit(x, y)) }.flatten();
    if let Some(hit) = hit {
        // **Pressed here, done on release**, as every button on this platform does: a press can be
        // taken back by dragging off the control, so a mis-aimed click is not a formatting edit.
        // SAFETY: one borrow; assigning a field dispatches nothing.
        unsafe {
            with_text(hwnd, |text| {
                text.pressed = Some(hit);
                // The pointer is over it by definition, and `button_up` asks whether it still
                // is — a press that arrives before any `WM_MOUSEMOVE` (the window opened under
                // the pointer) would otherwise be released onto a stale `None`.
                text.hover = Some(hit);
            });
            SetCapture(hwnd);
            let _ = SetFocus(Some(hwnd));
            let _ = InvalidateRect(Some(hwnd), None, false);
        }
        return;
    }
    let extend = mods().shift;
    // SAFETY: one borrow; the hit test is arithmetic over a layout and dispatches nothing.
    unsafe {
        with_text(hwnd, |text| {
            if let Some(at) = text.caret_at(x, y) {
                text.place(at, extend);
                text.dragging = true;
                text.caret_on = true;
            }
        });
        // Capture, so that a drag that leaves the window still reports where it went — the same
        // arrangement the grid has, and the reason `button_up` releases it on both panes.
        SetCapture(hwnd);
        let _ = SetFocus(Some(hwnd));
        let _ = InvalidateRect(Some(hwnd), None, false);
    }
}

/// A drag in progress. Nothing happens when no button is down: a plain move over a document has
/// to cost nothing, because it arrives on every pixel of travel.
fn text_mouse_move(hwnd: HWND, x: f64, y: f64) {
    // SAFETY: one borrow; nothing inside dispatches.
    let moved = unsafe {
        with_text(hwnd, |text| {
            // The strip's hover, repainted only when it *changes*. `WM_MOUSELEAVE` is not needed:
            // the pointer cannot leave the window from inside the strip without passing the rest
            // of it, and any point off a control clears the hover.
            let hover = text.page.strip_hit(x, y);
            let hovered = hover != text.hover;
            text.hover = hover;
            if !text.dragging {
                return hovered;
            }
            match text.caret_at(x, y) {
                Some(at) if at != text.caret => {
                    text.place(at, true);
                    true
                }
                _ => hovered,
            }
        })
    };
    if moved == Some(true) {
        // SAFETY: the borrow above has been released.
        unsafe {
            let _ = InvalidateRect(Some(hwnd), None, false);
        }
    }
}

/// A double click selects the word under it — the one gesture a text pane owes that a grid does
/// not, and it is [`text::keymap::word_boundary`] twice rather than a rule of its own.
fn text_double_click(hwnd: HWND, lparam: LPARAM) {
    let (x, y) = point(lparam);
    // SAFETY: one borrow; nothing inside dispatches.
    unsafe {
        with_text(hwnd, |text| {
            let Some(at) = text.caret_at(x, y) else {
                return;
            };
            let line = text.block_text(at.block);
            let start = text::keymap::word_boundary(&line, at.offset, false).unwrap_or(0);
            let end = text::keymap::word_boundary(&line, start, true)
                .unwrap_or_else(|| line.chars().count());
            text.anchor = Caret {
                block: at.block,
                offset: start,
            };
            text.caret = Caret {
                block: at.block,
                // A word motion forward stops at the *next* word's start, so trailing spaces
                // would come with it; the selection stops where the word does.
                offset: trim_trailing_spaces(&line, start, end),
            };
            text.goal_x = None;
            text.caret_on = true;
        });
        let _ = InvalidateRect(Some(hwnd), None, false);
    }
}

/// Where the word starting at `start` really ends, given that word motion stops at the beginning
/// of the next one.
fn trim_trailing_spaces(text: &str, start: usize, end: usize) -> usize {
    let chars: Vec<char> = text.chars().collect();
    let mut at = end.min(chars.len());
    while at > start && chars[at - 1].is_whitespace() {
        at -= 1;
    }
    at
}

/// A verb in the text pane, exhaustive on [`Command`] like the grid's. The spreadsheet's verbs are
/// no-ops here; the menu bar leaves them out over this pane, so only a stray accelerator reaches
/// them.
fn text_command(hwnd: HWND, command: Command) {
    match command {
        Command::NewSheet => new_document(hwnd, DocumentKind::Spreadsheet),
        Command::NewText => new_document(hwnd, DocumentKind::Text),
        Command::Welcome => welcome_screen(hwnd),
        Command::Open => open_document(hwnd),
        Command::Save => {
            save(hwnd);
        }
        Command::SaveAs => {
            save_as(hwnd);
        }
        Command::Exit => {
            // SAFETY: nothing is borrowed, and posting only queues the message.
            unsafe {
                let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
            }
        }
        Command::Undo => text_history(hwnd, true),
        Command::Redo => text_history(hwnd, false),
        Command::Cut => text_copy(hwnd, true),
        Command::Copy => text_copy(hwnd, false),
        Command::Paste => text_paste(hwnd),
        // Delete's verb. A selection is erased; with no selection there is nothing to clear,
        // because a document has no cells to empty.
        Command::ClearCells => text_erase(hwnd, true),
        Command::Bold => text_emphasise(hwnd, markdown::Emphasis::Bold),
        Command::Italic => text_emphasise(hwnd, markdown::Emphasis::Italic),
        Command::Underline => text_emphasise(hwnd, markdown::Emphasis::Underline),
        Command::Strike => text_emphasise(hwnd, markdown::Emphasis::Strike),
        Command::Code => text_emphasise(hwnd, markdown::Emphasis::Code),
        Command::PickFamily => text_pick_family(hwnd),
        Command::PickSize => text_pick_size(hwnd),
        Command::PickColor => text_pick_color(hwnd, false),
        Command::PickHighlight => text_pick_color(hwnd, true),
        Command::ClearFormatting => text_format(hwnd, grind_text::format::Change::Clear),
        Command::Paragraph => text_set_kind(hwnd, grind_text::BlockKind::Paragraph, None),
        Command::Title => text_set_kind(hwnd, grind_text::BlockKind::Paragraph, Some("Title")),
        Command::Subtitle => {
            text_set_kind(hwnd, grind_text::BlockKind::Paragraph, Some("Subtitle"))
        }
        Command::Heading1 => text_set_kind(hwnd, grind_text::BlockKind::Heading { level: 1 }, None),
        Command::Heading2 => text_set_kind(hwnd, grind_text::BlockKind::Heading { level: 2 }, None),
        Command::Heading3 => text_set_kind(hwnd, grind_text::BlockKind::Heading { level: 3 }, None),
        Command::GoTo => text_go_to(hwnd),
        Command::Find => text_find(hwnd),
        Command::FindNext => text_find_step(hwnd, grind_text::find::Towards::Next),
        Command::FindPrevious => text_find_step(hwnd, grind_text::find::Towards::Previous),
        Command::Replace => text_replace(hwnd),
        Command::Outline => text_outline(hwnd),
        Command::BlockKindDialog => text_block_kind_dialog(hwnd),
        Command::InsertPicture => text_insert_picture(hwnd),
        Command::AltText => text_alt_text(hwnd),
        Command::InsertTable => text_insert_table(hwnd),
        Command::Bookmark => text_bookmark(hwnd),
        Command::ParagraphStyle => text_paragraph_style(hwnd),
        Command::ParagraphUp => text_move_paragraphs(hwnd, true),
        Command::ParagraphDown => text_move_paragraphs(hwnd, false),
        Command::ParagraphDelete => text_delete_paragraphs(hwnd),
        Command::ImportMarkdown => text_import_markdown(hwnd),
        Command::ExportMarkdown => text_export_markdown(hwnd),
        Command::ExportPdf => text_export_pdf(hwnd),
        Command::PrintPreview => text_print_preview(hwnd),
        Command::NextMisspelling => text_next_misspelling(hwnd),
        Command::SpellingLanguage => text_spelling_language(hwnd),
        Command::PersonalDictionary => personal_dictionary(hwnd),
        Command::ToggleSpelling => text_toggle_spelling(hwnd),
        Command::Print => text_print(hwnd),
        Command::ShowSource => show_source(hwnd),
        Command::CheckDocument => check_document(hwnd),
        Command::ToggleNames => text_toggle_names(hwnd),
        Command::Shortcuts => show_shortcuts(hwnd),
        Command::About => dialog::about(hwnd),
        Command::Licences => dialog::licences(hwnd),
        // The spreadsheet's, and this pane has no answer to any of them: it has no sheets, no
        // cells for `CellRole` to classify, and no formulas to list, explain or read out.
        Command::Recalculate
        | Command::FillDown
        | Command::FillRight
        | Command::CopyValue
        | Command::FormulaToValue
        | Command::FillAcross
        | Command::MergeCells
        | Command::UnmergeCells
        | Command::InsertChart
        | Command::InsertBarChart
        | Command::InsertLineChart
        | Command::InsertPieChart
        | Command::ChartToBar
        | Command::ChartToLine
        | Command::ChartToPie
        | Command::PreviewChart
        | Command::DeleteChart
        | Command::RestyleChart
        | Command::MoveChart
        | Command::WrapText
        | Command::BordersAll
        | Command::BordersNone
        | Command::HideRows
        | Command::ShowRows
        | Command::HideColumns
        | Command::ShowColumns
        | Command::RowHeight
        | Command::FitColumns
        | Command::FitRows
        | Command::FitSheet
        | Command::ColumnWidth
        | Command::DefineName
        | Command::RenameName
        | Command::InlineName
        | Command::DeleteName
        | Command::DocumentLocale
        | Command::FindCalculation
        | Command::Evaluate
        | Command::SheetAdd
        | Command::SheetRename
        | Command::SheetDelete
        | Command::SheetNext
        | Command::SheetPrevious
        | Command::FunctionList
        | Command::ExplainFormula
        | Command::ToggleFriendly
        | Command::ToggleFilter
        | Command::FormatTable
        | Command::FormatTableTotals
        | Command::AddRule
        | Command::RemoveRule
        | Command::CurrencyEuro
        | Command::CurrencyDollar
        | Command::CurrencyPound
        | Command::GroupThousands
        // A cell's alignment, fill and number format: a run of text has none of the three.
        | Command::AlignLeft
        | Command::AlignCenter
        | Command::AlignRight
        | Command::IndentLess
        | Command::IndentMore
        | Command::TextRotation
        | Command::PickBackground
        | Command::NumberFormat
        | Command::FewerDecimals
        | Command::MoreDecimals
        // CSV is cells in both directions, so neither means anything here — and `applies_to`
        // keeps both out of this pane's File menu rather than leaving them to be no-ops.
        | Command::ImportCsv
        | Command::ImportCsvWith
        | Command::ExportCsv
        | Command::ToggleFormulas
        | Command::ZoomIn
        | Command::ZoomOut
        | Command::ZoomReset
        | Command::ToggleRoles => {}
    }
}

/// `doc/view-modes.md`'s name overlay, flipped — `:names`' equivalent for this pane. Menu-only,
/// like the grid's own two, and the menu is rebuilt straight after for the same reason
/// [`toggle_overlay`] rebuilds it.
fn text_toggle_names(hwnd: HWND) {
    // SAFETY: one borrow; nothing inside dispatches.
    unsafe {
        with_text(hwnd, |text| text.show_names = !text.show_names);
    }
    build_menu(hwnd);
    refresh(hwnd);
}

/// Go to an address — `p12`, `#intro` or `§2.1.3` — in a modal prompt, since this pane owns no
/// child control for a box; `loc::parse` and `App::resolve_caret` are the calls.
fn text_go_to(hwnd: HWND) {
    let Some(address) = dialog::prompt(hwnd, "Go To", "Address — p12, #bookmark or §2.1.3:", "")
    else {
        return;
    };
    // SAFETY: a fresh borrow, taken after the dialog rather than across it.
    let outcome = unsafe {
        with_text(hwnd, |text| {
            grind_text::loc::parse(&address)
                .map_err(|e| e.to_string())
                .and_then(|loc| text.app.resolve_caret(&loc).map_err(|e| e.to_string()))
                .map(|caret| {
                    text.place(caret, false);
                    text.caret_on = true;
                })
        })
    };
    match outcome {
        Some(Err(message)) => dialog::error(hwnd, &message),
        Some(Ok(())) => refresh(hwnd),
        None => {}
    }
}

/// Edit ▸ Find… over the page — ask for a word, then select the first occurrence at or after the
/// caret. The prompt runs a nested loop, so the word is read in one borrow and the search is a
/// second (decision 7). Case is ignored (`grind_text::find`).
fn text_find(hwnd: HWND) {
    // SAFETY: one borrow, released before the prompt.
    let Some(offered) = (unsafe { with_text(hwnd, |text| text.needle.clone()) }) else {
        return;
    };
    let Some(needle) = dialog::prompt(hwnd, "Find", "Find in the document:", &offered) else {
        return;
    };
    if needle.is_empty() {
        return;
    }
    // SAFETY: a fresh borrow, taken after the dialog has closed.
    unsafe {
        with_text(hwnd, |text| text.needle = needle);
    }
    text_find_step(hwnd, grind_text::find::Towards::Here);
}

/// F3 / Shift+F3, and Find…'s landing: the occurrences are asked for again each time, and the next
/// is selected so typing replaces it.
fn text_find_step(hwnd: HWND, towards: grind_text::find::Towards) {
    use grind_text::find;
    // SAFETY: one borrow, and no dialog on either side of it.
    let asked = unsafe {
        with_text(hwnd, |text| {
            if text.needle.is_empty() {
                return false;
            }
            let hits = find::hits(&text.app, &text.needle);
            // From the selection's start when there is one — usually the last hit — and the
            // caret otherwise.
            let (at, _) = text::keymap::ordered(text.anchor, text.caret);
            let said = match find::step(&hits, at, towards) {
                Some(index) => {
                    let from = hits[index];
                    text.place(from, false);
                    text.place(find::end_of(from, &text.needle), true);
                    text.caret_on = true;
                    notice::text_found(index, hits.len(), &text.needle)
                }
                None => notice::text_not_found(&text.needle),
            };
            text.say(Some(said));
            true
        })
    };
    match asked {
        // Nothing searched for yet: F3 means "find", which is where the word comes from.
        Some(false) => text_find(hwnd),
        Some(true) => text_refresh(hwnd),
        None => {}
    }
}

/// Edit ▸ Replace… — what, then with what, then `App::replace` over the whole document in one
/// undo step. Two prompts, as the grid's is; neither is asked with anything borrowed. Exact in
/// case, since it writes: only finding is lenient.
fn text_replace(hwnd: HWND) {
    // SAFETY: one borrow, released before the prompts.
    let Some(offered) = (unsafe { with_text(hwnd, |text| text.needle.clone()) }) else {
        return;
    };
    let Some(what) = dialog::prompt(hwnd, "Replace", "Replace what:", &offered) else {
        return;
    };
    if what.is_empty() {
        return;
    }
    let Some(with) = dialog::prompt(hwnd, "Replace", &format!("Replace “{what}” with:"), "")
    else {
        return;
    };
    // SAFETY: a fresh borrow, taken after both dialogs have closed.
    unsafe {
        with_text(hwnd, |text| {
            let said = match text.app.replace(&what, &with) {
                Ok(blocks) => notice::text_replaced(blocks, &what),
                Err(error) => error.to_string(),
            };
            text.needle = what;
            text.say(Some(said));
        });
    }
    text_refresh(hwnd);
}

/// The outline dialog: every heading, indented by its depth, each a jump — straight to the block
/// index, rather than round-tripping the `§2.1.3` address through `loc::parse`.
fn text_outline(hwnd: HWND) {
    // SAFETY: one borrow, released before the dialog — which runs a nested message loop.
    let headings = unsafe { with_text(hwnd, |text| text.app.outline()) }.unwrap_or_default();
    if headings.is_empty() {
        dialog::error(hwnd, "This document has no headings.");
        return;
    }
    let items: Vec<String> = headings
        .iter()
        .map(|heading| {
            let indent = "    ".repeat(heading.path.len().saturating_sub(1));
            format!("{indent}{}  {}", heading.address(), heading.text)
        })
        .collect();
    let Some(choice) = dialog::choose(hwnd, "Outline", &items, 0) else {
        return;
    };
    let Some(heading) = headings.get(choice) else {
        return;
    };
    let block = heading.index;
    // SAFETY: a fresh borrow, taken after the dialog rather than across it.
    unsafe {
        with_text(hwnd, |text| {
            text.place(Caret { block, offset: 0 }, false);
            text.caret_on = true;
        });
    }
    refresh(hwnd);
}

/// Which emphasis each of the strip's five toggles is, left to right — [`text::geom::Page::strip_buttons`]'s
/// order and this array's are the one place that ordering is written down.
const STRIP_BUTTONS: [markdown::Emphasis; 5] = [
    markdown::Emphasis::Bold,
    markdown::Emphasis::Italic,
    markdown::Emphasis::Underline,
    markdown::Emphasis::Strike,
    markdown::Emphasis::Code,
];

/// The style a toggle reads before it draws, and what every control changes:
/// `grind_text::format::here` — the selection's agreed style, or with nothing selected what the
/// next character would carry.
fn text_style_here(text: &Text) -> grind_text::CharStyle {
    let selection = text.has_selection().then(|| text.range());
    grind_text::format::here(&text.app, selection, text.caret, text.resume.as_ref())
}

/// Which of the strip's five toggles should be drawn pressed in, for [`text::draw::paint`].
fn format_state(text: &Text) -> [bool; 5] {
    let style = text_style_here(text);
    STRIP_BUTTONS.map(|emphasis| grind_text::format::has(&style, emphasis))
}

/// Toggle one emphasis across the selection — the strip, Ctrl+B/I/U and the Format menu all
/// converge here: on across it, or off when every character has it (`Change::toggle`).
fn text_emphasise(hwnd: HWND, emphasis: markdown::Emphasis) {
    // SAFETY: one borrow. `set_char_style` notifies, and the observer posts rather than sends.
    let style = unsafe { with_text(hwnd, |text| text_style_here(text)) }.unwrap_or_default();
    text_format(hwnd, grind_text::format::Change::toggle(emphasis, &style));
}

/// Apply a format change over the selection — or, with nothing selected, hold it for the next
/// character typed (`Text::resume`): Bold, then type, is bold. `grind_text::format::apply` is the
/// rule.
fn text_format(hwnd: HWND, change: grind_text::format::Change) {
    // SAFETY: one borrow. `set_char_style` notifies, and the observer posts rather than sends.
    unsafe {
        with_text(hwnd, |text| {
            let selection = text.has_selection().then(|| text.range());
            let landed = grind_text::format::apply(
                &text.app,
                selection,
                text.caret,
                text.resume.as_ref(),
                &change,
            );
            match landed {
                Ok(grind_text::format::Landed::Written) => text.say(None),
                Ok(grind_text::format::Landed::Pending(style)) => {
                    text.resume = Some(style);
                    text.say(None);
                }
                Err(error) => text.say(Some(error.to_string())),
            }
        });
    }
    refresh(hwnd);
}

/// The families the picker offers — curated, not enumerated from the system (a named gap in
/// `doc/windows-shell.md`). A document's own family is added when it is not in the list, so a
/// document written elsewhere shows what it chose.
const FONT_CHOICES: [&str; 8] = [
    "Segoe UI",
    "Calibri",
    "Arial",
    "Times New Roman",
    "Georgia",
    "Verdana",
    "Consolas",
    "Courier New",
];

/// *Font* — `dialog::choose` over [`FONT_CHOICES`], [`grind_text::format::DEFAULT`] first.
fn text_pick_family(hwnd: HWND) {
    let style = unsafe { with_text(hwnd, |text| text_style_here(text)) }.unwrap_or_default();
    let mut items: Vec<String> = std::iter::once(grind_text::format::DEFAULT.to_owned())
        .chain(FONT_CHOICES.iter().map(|name| (*name).to_owned()))
        .collect();
    if let Some(current) = style.font_family.as_deref()
        && !items.iter().any(|item| item == current)
    {
        items.insert(1, current.to_owned());
    }
    let initial = style
        .font_family
        .as_deref()
        .and_then(|current| items.iter().position(|item| item == current))
        .unwrap_or(0);
    let Some(choice) = dialog::choose(hwnd, "Font", &items, initial) else {
        return;
    };
    let value = (choice != 0).then(|| items[choice].clone());
    text_format(hwnd, grind_text::format::Change::Family(value));
}

/// *Size* — `dialog::choose` over [`grind_text::format::sizes`], the same ladder
/// `grind-text-gtk`'s drop-down offers.
fn text_pick_size(hwnd: HWND) {
    let style = unsafe { with_text(hwnd, |text| text_style_here(text)) }.unwrap_or_default();
    let items = grind_text::format::sizes(style.font_size.as_deref());
    let initial = style
        .font_size
        .as_deref()
        .and_then(|current| items.iter().position(|item| item == current))
        .unwrap_or(0);
    let Some(choice) = dialog::choose(hwnd, "Font Size", &items, initial) else {
        return;
    };
    let value = (choice != 0).then(|| items[choice].clone());
    text_format(hwnd, grind_text::format::Change::Size(value));
}

/// Either swatch — `grind_core::style::PALETTE` as the choices, *Automatic* first, over the same
/// `dialog::choose` popup a colour name is one row of.
fn text_pick_color(hwnd: HWND, highlight: bool) {
    let style = unsafe { with_text(hwnd, |text| text_style_here(text)) }.unwrap_or_default();
    let current = match highlight {
        true => style.background.clone(),
        false => style.color.clone(),
    };
    let items = crate::strip::colour_choices();
    let initial = crate::strip::colour_row(current.as_deref());
    let title = match highlight {
        true => "Highlight",
        false => "Text Colour",
    };
    let Some(choice) = dialog::choose(hwnd, title, &items, initial) else {
        return;
    };
    let value = (choice != 0).then(|| grind_core::style::PALETTE[choice - 1].1.to_owned());
    let change = match highlight {
        true => grind_text::format::Change::Highlight(value),
        false => grind_text::format::Change::Color(value),
    };
    text_format(hwnd, change);
}

/// Turn the caret's block into this kind, wearing this named paragraph style — `App::set_kind` and
/// `App::set_style` in one gesture. **The style is only taken off when this window put it on**
/// (`grind_text::named_style_for`): Paragraph after *Title* or *Subtitle* clears the name, and a
/// document's own `Quotations` stays. A block's kind is asked of the caret, not a selection, as
/// `App::set_kind` takes an index.
fn text_set_kind(hwnd: HWND, kind: grind_text::BlockKind, style: Option<&str>) {
    // SAFETY: one borrow. `set_kind`/`set_style` notify, and the observer posts rather than
    // sends.
    unsafe {
        with_text(hwnd, |text| {
            let index = text.caret.block;
            let viewport = text.app.get_viewport(index..index + 1);
            let Some(block) = viewport.get(index) else {
                return;
            };
            let was = block.style.clone();
            let wanted = grind_text::named_style_for(was.as_deref(), style);
            let kind_changed = block.kind != kind;
            if kind_changed && let Err(error) = text.app.set_kind(index, kind) {
                text.say(Some(error.to_string()));
                return;
            }
            if wanted != was {
                match text.app.set_style(index..index + 1, wanted) {
                    Ok(_) => text.say(None),
                    Err(error) => text.say(Some(error.to_string())),
                }
            } else if kind_changed {
                text.say(None);
            }
        });
    }
    refresh(hwnd);
}

/// Tab or Shift+Tab: nest a list item, un-nest one, or start one at a block's front
/// (`grind_text::indent_kind`); a literal tab mid-sentence. Shift+Tab with nothing to un-nest
/// types nothing.
fn text_indent(hwnd: HWND, back: bool) {
    let by = if back { -1 } else { 1 };
    // SAFETY: one borrow. `set_kind` notifies, and the observer posts rather than sends.
    let handled = unsafe {
        with_text(hwnd, |text| {
            let index = text.caret.block;
            let viewport = text.app.get_viewport(index..index + 1);
            let Some(block) = viewport.get(index) else {
                return false;
            };
            let Some(kind) = grind_text::indent_kind(&block.kind, text.caret.offset, by) else {
                return false;
            };
            if let Err(error) = text.app.set_kind(index, kind) {
                text.say(Some(error.to_string()));
            }
            true
        })
    };
    match handled {
        Some(true) => refresh(hwnd),
        _ if !back => text_insert(hwnd, "\t"),
        _ => {}
    }
}

/// The deepest list item the block-kind chooser offers. Every kind is listed rather than a key per
/// depth — headings past level 3 and lists with depths — in the `LISTBOX` `text_outline` opens.
/// Six heading levels, four list depths and Paragraph are the authoring ceiling.
const MAX_LIST_DEPTH: u32 = 4;

fn text_block_kind_dialog(hwnd: HWND) {
    let mut kinds = vec![grind_text::BlockKind::Paragraph];
    kinds.extend((1..=6).map(|level| grind_text::BlockKind::Heading { level }));
    kinds.extend((1..=MAX_LIST_DEPTH).map(|depth| grind_text::BlockKind::ListItem { depth }));
    let items: Vec<String> = kinds
        .iter()
        .map(|kind| match kind {
            grind_text::BlockKind::Paragraph => "Paragraph".to_owned(),
            grind_text::BlockKind::Heading { level } => format!("Heading {level}"),
            grind_text::BlockKind::ListItem { depth } => format!("List item, depth {depth}"),
        })
        .collect();
    let Some(choice) = dialog::choose(hwnd, "Block Kind", &items, 0) else {
        return;
    };
    let Some(kind) = kinds.get(choice).cloned() else {
        return;
    };
    text_set_kind(hwnd, kind, None);
}

/// Insert a picture — a file dialog, then `App::insert_image`, in a paragraph of its own: an empty
/// block as it stands, else a fresh paragraph after it, since a picture mid-sentence draws as the
/// placeholder character.
fn text_insert_picture(hwnd: HWND) {
    let Some(path) = dialog::open_image_path(hwnd) else {
        return;
    };
    let data = match std::fs::read(&path) {
        Ok(data) => data,
        Err(error) => {
            dialog::error(hwnd, &format!("Could not read {}: {error}", path.display()));
            return;
        }
    };
    let mime = image::mime_of(&path);
    // SAFETY: one borrow. `insert`/`insert_image` notify, and the observer posts rather than
    // sends.
    let outcome = unsafe {
        with_text(hwnd, |text| {
            grind_text::picture::insert_below(&text.app, text.caret.block, &mime, data)
                .map_err(|error| error.to_string())
        })
    };
    match outcome {
        // Past the picture, which is one caret position — so the next thing typed is a caption
        // rather than text wrapped around a frame nothing lays out yet.
        Some(Ok(at)) => {
            // SAFETY: a fresh borrow, taken after the one above released.
            unsafe {
                with_text(hwnd, |text| {
                    text.place(
                        Caret {
                            block: at,
                            offset: 1,
                        },
                        false,
                    );
                    text.caret_on = true;
                });
            }
            refresh(hwnd);
        }
        Some(Err(message)) => dialog::error(hwnd, &message),
        None => {}
    }
}

/// Format ▸ Picture Alt Text… — the picture beside the caret (`grind_text::picture::at`), its
/// alternative text asked for in `dialog::alt_text`, written by `picture::set_alt`. Read first,
/// dialog second, write third: the dialog runs a nested message loop (decision 7).
fn text_alt_text(hwnd: HWND) {
    // SAFETY: one borrow, released before the dialog.
    let here = unsafe {
        with_text(hwnd, |text| {
            let block = text.caret.block;
            let viewport = text.app.get_viewport(block..block + 1);
            let (offset, image) = grind_text::picture::at(viewport.get(block)?, text.caret.offset)?;
            Some((Caret { block, offset }, image.clone()))
        })
    }
    .flatten();
    let Some((at, image)) = here else {
        // SAFETY: one borrow.
        unsafe {
            with_text(hwnd, |text| {
                text.say(Some("Put the caret beside a picture first.".to_owned()))
            });
        }
        refresh(hwnd);
        return;
    };
    let Some((title, description)) = dialog::alt_text(
        hwnd,
        &image.data,
        image.title.as_deref().unwrap_or(""),
        image.description.as_deref().unwrap_or(""),
    ) else {
        return;
    };
    // SAFETY: a fresh borrow, after the dialog.
    let outcome = unsafe {
        with_text(hwnd, |text| {
            let done = grind_text::picture::set_alt(&text.app, at, &title, &description);
            if done.is_ok() {
                text.say(Some(match title.trim().is_empty() {
                    true => "The picture has no alt text.".to_owned(),
                    false => "Alt text saved.".to_owned(),
                }));
            }
            done
        })
    };
    match outcome {
        Some(Err(error)) => dialog::error(hwnd, &error.to_string()),
        Some(Ok(())) => refresh(hwnd),
        None => {}
    }
}

/// Format ▸ Insert Table… — one prompt for the size (`3x4`, `grind_text::table::parse_size`), then
/// the table below the caret's block, the caret on its first cell. Prompt first, borrow second:
/// the dialog runs a nested message loop (decision 7).
fn text_insert_table(hwnd: HWND) {
    let Some(answer) = dialog::prompt(hwnd, "Insert Table", "Size — rows × columns:", "3x3")
    else {
        return;
    };
    let Some((rows, columns)) = grind_text::table::parse_size(&answer) else {
        return dialog::error(hwnd, "A table size looks like 3x4 — rows, then columns.");
    };
    // SAFETY: one borrow, after the dialog. `insert_table` notifies; the observer posts.
    let outcome = unsafe {
        with_text(hwnd, |text| {
            grind_text::table::insert_below(&text.app, text.caret.block, rows, columns).map(|at| {
                text.place(
                    Caret {
                        block: at,
                        offset: 0,
                    },
                    false,
                );
                text.caret_on = true;
            })
        })
    };
    match outcome {
        Some(Err(error)) => dialog::error(hwnd, &error.to_string()),
        Some(Ok(())) => refresh(hwnd),
        None => {}
    }
}

/// Format ▸ Bookmark Here… — a name, then `App::set_bookmark` at the caret's block. The name
/// overlay goes on, so the anchor can be seen.
fn text_bookmark(hwnd: HWND) {
    let Some(name) = dialog::prompt(hwnd, "Bookmark", "Bookmark name:", "") else {
        return;
    };
    let name = name.trim().trim_start_matches('#').to_owned();
    if name.is_empty() {
        return;
    }
    // SAFETY: one borrow, after the dialog.
    let outcome = unsafe {
        with_text(hwnd, |text| {
            text.app
                .set_bookmark(&name, Some(text.caret.block))
                .map(|moved| {
                    text.show_names = true;
                    let said = match moved {
                        true => format!("#{name} moved here."),
                        false => format!("#{name} anchored here."),
                    };
                    text.say(Some(said));
                })
        })
    };
    match outcome {
        Some(Err(error)) => dialog::error(hwnd, &error.to_string()),
        Some(Ok(())) => {
            build_menu(hwnd);
            refresh(hwnd);
        }
        None => {}
    }
}

/// Format ▸ Paragraph Style Name… — `App::set_style` over the blocks the selection touches, an
/// empty answer removing the name.
fn text_paragraph_style(hwnd: HWND) {
    // SAFETY: one borrow, released before the prompt.
    let Some(current) = (unsafe {
        with_text(hwnd, |text| {
            let (from, _) = text::keymap::ordered(text.anchor, text.caret);
            text.app
                .get_viewport(from.block..from.block + 1)
                .get(0)
                .and_then(|block| block.style.clone())
                .unwrap_or_default()
        })
    }) else {
        return;
    };
    let Some(name) = dialog::prompt(
        hwnd,
        "Paragraph Style",
        "Style name (empty removes):",
        &current,
    ) else {
        return;
    };
    let name = name.trim().to_owned();
    // SAFETY: a fresh borrow, after the dialog.
    let outcome = unsafe {
        with_text(hwnd, |text| {
            let (from, to) = text::keymap::ordered(text.anchor, text.caret);
            text.app.set_style(
                from.block..to.block + 1,
                (!name.is_empty()).then(|| name.clone()),
            )
        })
    };
    match outcome {
        Some(Err(error)) => dialog::error(hwnd, &error.to_string()),
        Some(Ok(_)) => refresh(hwnd),
        None => {}
    }
}

/// The blocks a paragraph verb acts on: every one the selection touches, or the caret's.
fn text_blocks(text: &Text) -> std::ops::RangeInclusive<usize> {
    let (from, to) = text::keymap::ordered(text.anchor, text.caret);
    from.block..=to.block
}

/// Format ▸ Move Paragraph Up / Down — `grind_text::blocks::shift`, the caret and selection going
/// with the paragraphs.
fn text_move_paragraphs(hwnd: HWND, up: bool) {
    // SAFETY: one borrow, no dialog.
    unsafe {
        with_text(hwnd, |text| {
            match grind_text::blocks::shift(&text.app, text_blocks(text), up) {
                Ok(()) => {
                    let step = |caret: &mut Caret| match up {
                        true => caret.block -= 1,
                        false => caret.block += 1,
                    };
                    step(&mut text.caret);
                    step(&mut text.anchor);
                    text.say(None);
                }
                Err(why) => text.say(Some(why)),
            }
        });
    }
    refresh(hwnd);
}

/// Format ▸ Delete Paragraph — `grind_text::blocks::remove`, the caret at the start of what
/// followed.
fn text_delete_paragraphs(hwnd: HWND) {
    // SAFETY: one borrow, no dialog.
    unsafe {
        with_text(hwnd, |text| {
            match grind_text::blocks::remove(&text.app, text_blocks(text)) {
                Ok(block) => {
                    text.place(Caret { block, offset: 0 }, false);
                    text.caret_on = true;
                    text.say(None);
                }
                Err(why) => text.say(Some(why)),
            }
        });
    }
    refresh(hwnd);
}

/// File ▸ Import Markdown… — a markdown file read in before the caret's block, one undo step
/// (`App::import_markdown`). The dialog runs a nested message loop, so nothing is borrowed across
/// it (decision 7).
fn text_import_markdown(hwnd: HWND) {
    let Some(path) = dialog::open_markdown_path(hwnd) else {
        return;
    };
    let markdown = match std::fs::read(&path)
        .map_err(|error| error.to_string())
        .and_then(|bytes| String::from_utf8(bytes).map_err(|_| "not UTF-8".to_owned()))
    {
        Ok(markdown) => markdown,
        Err(why) => {
            return dialog::error(
                hwnd,
                &format!("Could not read {}:\n\n{why}", path.display()),
            );
        }
    };
    // SAFETY: one borrow, taken after the dialog has closed.
    unsafe {
        with_text(hwnd, |text| {
            let block = text.caret.block;
            let resolve = grind_text::commonmark::beside(&path);
            match text.app.import_markdown(block, &markdown, &resolve) {
                Ok(count) => {
                    text.place(Caret { block, offset: 0 }, false);
                    text.caret_on = true;
                    text.say(Some(format!("Imported {count} block(s) from Markdown.")));
                }
                Err(error) => text.say(Some(error.to_string())),
            }
        });
    }
    refresh(hwnd);
}

/// File ▸ Export Markdown… — the blocks the selection touches, or the whole document when there is
/// no selection, written as CommonMark. **Writes a file and changes nothing**: no undo entry, no
/// dirty flag.
fn text_export_markdown(hwnd: HWND) {
    // SAFETY: one borrow, released before the dialog.
    let suggested = unsafe {
        with_text(hwnd, |text| {
            let stem = text
                .path
                .as_deref()
                .or(text.imported.as_deref())
                .and_then(Path::file_stem)
                .map(|stem| stem.to_string_lossy().into_owned());
            format!("{}.md", stem.unwrap_or_else(|| "Untitled".to_owned()))
        })
    }
    .unwrap_or_else(|| "Untitled.md".to_owned());
    let Some(path) = dialog::save_markdown_path(hwnd, &suggested) else {
        return;
    };
    // SAFETY: one borrow, taken after the dialog has closed.
    let written = unsafe {
        with_text(hwnd, |text| {
            let blocks = match text.anchor == text.caret {
                true => 0..text.app.block_count(),
                false => {
                    let blocks = text_blocks(text);
                    *blocks.start()..*blocks.end() + 1
                }
            };
            // The pictures go in a directory beside it (`notes.images\`), named for their bytes.
            let pictures = grind_text::commonmark::Pictures::beside(&path.display().to_string());
            text.app
                .export_markdown(blocks, &pictures)
                .map_err(|error| error.to_string())
        })
    };
    let exported = match written {
        Some(Ok(exported)) => exported,
        Some(Err(why)) => return dialog::error(hwnd, &why),
        None => return,
    };
    let said = match exported.save(&path) {
        Ok(said) => said,
        Err(error) => return dialog::error(hwnd, &format!("Could not write it:\n\n{error}")),
    };
    // SAFETY: one borrow, no dialog.
    unsafe {
        with_text(hwnd, |text| {
            text.say(Some(said));
        });
    }
    refresh(hwnd);
}

/// File ▸ Export PDF… — the document typeset and written as a PDF (`doc/pdf-export.md`); the
/// notice bar is the export's own report, which names any family it had to substitute.
fn text_export_pdf(hwnd: HWND) {
    // SAFETY: one borrow, released before the dialog.
    let stem = unsafe {
        with_text(hwnd, |text| {
            text.path
                .as_deref()
                .or(text.imported.as_deref())
                .and_then(Path::file_stem)
                .map(|stem| stem.to_string_lossy().into_owned())
        })
    }
    .flatten()
    .unwrap_or_else(|| "Untitled".to_owned());
    let Some(path) = dialog::save_pdf_path(hwnd, &format!("{stem}.pdf")) else {
        return;
    };
    // SAFETY: one borrow, taken after the dialog has closed.
    let exported = unsafe {
        with_text(hwnd, |text| {
            let options = grind_print::Options {
                paper: None,
                title: Some(stem.clone()),
            };
            grind_print::export(&text.app, grind_print::fonts_for(&text.app), &options)
        })
    };
    let (bytes, report) = match exported {
        Some(Ok(done)) => done,
        Some(Err(why)) => return dialog::error(hwnd, &why),
        None => return,
    };
    if let Err(error) = grind_core::atomic::write(&path, bytes) {
        return dialog::error(
            hwnd,
            &format!("Could not write {}:\n\n{error}", path.display()),
        );
    }
    // SAFETY: one borrow, no dialog.
    unsafe {
        with_text(hwnd, |text| {
            text.say(Some(format!(
                "Wrote {} — {}",
                path.display(),
                report.summary()
            )));
        });
    }
    refresh(hwnd);
}

/// The document typeset once, for the preview and the printer: the pages as display lists, the
/// typesetter whose faces they name, the paper, and the document's name.
fn text_typeset(
    hwnd: HWND,
) -> Option<(
    grind_print::ops::Document,
    grind_print::Typesetter,
    grind_core::page::PageGeometry,
    String,
)> {
    // SAFETY: one borrow, no dialog.
    unsafe {
        with_text(hwnd, |text| {
            let name = text
                .path
                .as_deref()
                .or(text.imported.as_deref())
                .and_then(Path::file_stem)
                .map_or_else(
                    || "Untitled".to_owned(),
                    |s| s.to_string_lossy().into_owned(),
                );
            let options = grind_print::Options::default();
            let (doc, setter) =
                grind_print::typeset(&text.app, grind_print::fonts_for(&text.app), &options);
            (doc, setter, text.app.page().unwrap_or_default(), name)
        })
    }
}

/// File ▸ Print Preview — every page as it prints (`doc/pdf-export.md` §4); Print… in it prints.
fn text_print_preview(hwnd: HWND) {
    let Some((doc, setter, paper, _)) = text_typeset(hwnd) else {
        return;
    };
    let count = doc.pages.len();
    let size = doc
        .pages
        .first()
        .map_or((595.0, 842.0), |page| (page.width, page.height));
    let render = Box::new(move |page: usize, scale: f32| {
        let page = doc.pages.get(page)?;
        let raster = grind_print::raster::render(page, setter.fonts(), scale);
        Some((raster.width, raster.height, raster.rgba))
    });
    let title = Box::new(move |page: usize| text::paper::title(page, count, &paper));
    if dialog::page_preview(hwnd, count, size, render, title) {
        text_print(hwnd);
    }
}

/// File ▸ Print… — the system's print dialog, then every page as a raster at the printer's
/// resolution (at most [`text::paper::PRINT_DPI`]), scaled onto the printable area.
fn text_print(hwnd: HWND) {
    use windows::Win32::Graphics::Gdi::{DeleteDC, GetDeviceCaps, HORZRES, LOGPIXELSX, VERTRES};
    use windows::Win32::Storage::Xps::{DOCINFOW, EndDoc, EndPage, StartDocW, StartPage};
    use windows::Win32::UI::Controls::Dialogs::{
        PD_RETURNDC, PD_USEDEVMODECOPIESANDCOLLATE, PRINTDLGW, PrintDlgW,
    };
    let Some((doc, setter, _, name)) = text_typeset(hwnd) else {
        return;
    };
    let mut request = PRINTDLGW {
        lStructSize: u32::try_from(std::mem::size_of::<PRINTDLGW>()).expect("small"),
        hwndOwner: hwnd,
        Flags: PD_RETURNDC | PD_USEDEVMODECOPIESANDCOLLATE,
        nCopies: 1,
        ..Default::default()
    };
    // SAFETY: `request` is initialised and lives across the call. **A nested message loop**;
    // nothing borrowed.
    if !unsafe { PrintDlgW(&mut request) }.as_bool() || request.hDC.is_invalid() {
        return;
    }
    let dc = request.hDC;
    let title = gdi::wide(&name);
    // SAFETY: `dc` is the printer DC the dialog returned, deleted at the end; every buffer lives
    // across the calls that read it.
    let printed = unsafe {
        let dpi = u32::try_from(GetDeviceCaps(Some(dc), LOGPIXELSX)).unwrap_or(0);
        let area = (
            GetDeviceCaps(Some(dc), HORZRES),
            GetDeviceCaps(Some(dc), VERTRES),
        );
        let info = DOCINFOW {
            cbSize: i32::try_from(std::mem::size_of::<DOCINFOW>()).expect("small"),
            lpszDocName: PCWSTR(title.as_ptr()),
            ..Default::default()
        };
        let mut ok = StartDocW(dc, &info) > 0;
        for page in &doc.pages {
            if !ok {
                break;
            }
            let raster =
                grind_print::raster::render(page, setter.fonts(), text::paper::print_scale(dpi));
            // The printable area at the page's own proportions, from its top-left corner.
            let fit = (area.0 as f32 / page.width).min(area.1 as f32 / page.height);
            let dest = (0, 0, (page.width * fit) as i32, (page.height * fit) as i32);
            ok = StartPage(dc) > 0
                && gdi::print_image(
                    dc,
                    dest,
                    (raster.width, raster.height),
                    &text::paper::bgra(&raster.rgba),
                )
                && EndPage(dc) > 0;
        }
        if ok {
            ok = EndDoc(dc) > 0;
        }
        let _ = DeleteDC(dc);
        ok
    };
    // SAFETY: one borrow, no dialog.
    unsafe {
        with_text(hwnd, |text| {
            text.say(Some(match printed {
                true => format!("Sent {} page(s) to the printer.", doc.pages.len()),
                false => "The printer did not take the document.".to_owned(),
            }));
        });
    }
    refresh(hwnd);
}

fn text_history(hwnd: HWND, undo: bool) {
    // SAFETY: one borrow. `App::undo` notifies, and the observer posts rather than sends.
    unsafe {
        with_text(hwnd, |text| {
            let did = match undo {
                true => text.app.undo(),
                false => text.app.redo(),
            };
            if did {
                // Whatever the banner said was about the document as it was — and the caret may
                // now be past the end of a block that shrank.
                text.say(None);
                let block = text
                    .caret
                    .block
                    .min(text.app.block_count().saturating_sub(1));
                let offset = text.caret.offset.min(text.block_len(block));
                text.place(Caret { block, offset }, false);
            }
        });
    }
    refresh(hwnd);
}

/// Copy the selection as `CF_UNICODETEXT`, and with `cut` erase it. The document's **plain text**
/// travels, one `\r\n` per block — what every program here reads. Formatting does not, there being
/// no `CF_RTF` or `HTML Format` writer: a named gap.
fn text_copy(hwnd: HWND, cut: bool) {
    // SAFETY: one borrow, released before the clipboard is opened — `clipboard.rs` is the only
    // file that opens it and it does so with nothing else borrowed.
    let copied = unsafe {
        with_text(hwnd, |text| {
            if !text.has_selection() {
                return None;
            }
            let (from, to) = text.range();
            let mut out = String::new();
            for block in from.block..=to.block.min(text.app.block_count().saturating_sub(1)) {
                let line: Vec<char> = text.block_text(block).chars().collect();
                let start = match block == from.block {
                    true => from.offset.min(line.len()),
                    false => 0,
                };
                let end = match block == to.block {
                    true => to.offset.min(line.len()),
                    false => line.len(),
                };
                if block > from.block {
                    out.push_str("\r\n");
                }
                out.extend(&line[start.min(end)..end]);
            }
            Some(out)
        })
    }
    .flatten();
    let Some(text) = copied else { return };
    clipboard::set_text(hwnd, &text);
    if cut {
        // SAFETY: a fresh borrow, taken after the clipboard call rather than across it.
        unsafe {
            with_text(hwnd, |pane| {
                text_drop_selection(pane);
            });
        }
        refresh(hwnd);
    }
}

/// Paste text at the caret. A newline in what arrives becomes a block break, which is what makes
/// pasting a paragraph of prose produce paragraphs rather than one line with `\n` in it.
fn text_paste(hwnd: HWND) {
    let Some(what) = clipboard::get_text(hwnd) else {
        return;
    };
    // `\r\n` and a bare `\n` both mean the same thing, and a document that came from any other
    // program may carry either.
    let lines: Vec<&str> = what
        .split('\n')
        .map(|line| line.trim_end_matches('\r'))
        .collect();
    // SAFETY: one borrow. Every call inside notifies, and the observer posts rather than sends.
    unsafe {
        with_text(hwnd, |text| {
            text_drop_selection(text);
            for (index, line) in lines.iter().enumerate() {
                if index > 0 {
                    let at = text.caret;
                    if text.app.split_block(at).is_err() {
                        break;
                    }
                    text.caret = Caret {
                        block: at.block + 1,
                        offset: 0,
                    };
                }
                if line.is_empty() {
                    continue;
                }
                let at = text.caret;
                if text.app.insert_text(at, line).is_err() {
                    break;
                }
                text.caret = Caret {
                    block: at.block,
                    offset: at.offset + line.chars().count(),
                };
            }
            text.place(text.caret, false);
        });
    }
    refresh(hwnd);
}

/// Everything one frame of the text pane needs, drawn onto a device context — an `HDC` and the
/// state, so [`render`] is a second caller. `system_caret` is `false` only for [`render`]'s
/// windowless frame; elsewhere Windows draws its own caret, and drawing one here too would be two.
fn draw_text_frame(dc: HDC, state: &Text, system_caret: bool) {
    let body = state.page.body();
    let visible = state
        .flow
        .visible(state.page.scroll, state.page.scroll + body.h);
    // Exactly the blocks on screen reach `get_viewport`, which is architecture rule 1: no getter
    // hands out the whole document.
    let range = match (visible.first(), visible.last()) {
        (Some(first), Some(last)) => first.index..last.index + 1,
        _ => 0..0,
    };
    let viewport = state.app.get_viewport(range);
    let Some(faces) = state.faces() else { return };
    let mut blocks = Vec::with_capacity(visible.len());
    for slot in visible {
        let Some(view) = viewport.get(slot.index) else {
            continue;
        };
        let (width, metrics) =
            grind_text::Faces::of(&faces, slot.index, &view.kind, view.style.as_deref());
        let Ok(layout) = state.app.layout_block(slot.index, width, metrics) else {
            continue;
        };
        blocks.push(text::draw::Painted {
            slot: *slot,
            view,
            layout,
        });
    }
    let here = state
        .app
        .get_viewport(state.caret.block..state.caret.block + 1)
        .get(state.caret.block)
        .map(|block| text::status::describe(&block.kind, block.style.as_deref()))
        .unwrap_or_default();
    // The language is the picker's to say, at the bar's end (`Page::status_picker`).
    let status = text::status::status_line(&here, selected_chars(state), state.app.counts());
    // The word still being typed is not underlined: the caret at its end with nothing selected
    // is somebody halfway through it, and every word processor waits.
    let (from, to) = state.range();
    let misspelt: Vec<grind_text::Misspelling> = match (blocks.first(), blocks.last()) {
        (Some(first), Some(last)) => state
            .app
            .misspellings(first.slot.index..last.slot.index + 1)
            .into_iter()
            .filter(|m| from != to || !m.being_typed(state.caret))
            .collect(),
        _ => Vec::new(),
    };
    let style = text_style_here(state);
    text::draw::paint(
        dc,
        &text::draw::Frame {
            page: &state.page,
            theme: state.theme,
            faces: &faces,
            blocks: &blocks,
            cells: state.flow.cells(),
            height: state.flow.height(),
            selection: state.range(),
            caret: state.caret,
            caret_on: state.caret_on && !system_caret,
            status: &status,
            status_picker: state.spelling.chip(false),
            banner: state.banner.as_deref(),
            font_px: scale(theme::text::CAPTION, state.page.dpi).round() as i32,
            body_px: scale(theme::text::BODY, state.page.dpi).round() as i32,
            face: face(),
            hover: state.hover,
            pressed: state.pressed,
            format: format_state(state),
            family: style.font_family.as_deref(),
            size: style.font_size.as_deref(),
            color: style.color.as_deref(),
            highlight: style.background.as_deref(),
            names: state.show_names,
            misspelt: &misspelt,
        },
    );
}

/// How many characters the selection covers, for the status bar — counted, since a selection
/// across blocks covers the breaks too, each a character a `Caret` can sit either side of.
fn selected_chars(state: &Text) -> usize {
    let (from, to) = state.range();
    if from == to {
        return 0;
    }
    if from.block == to.block {
        return to.offset.saturating_sub(from.offset);
    }
    let mut total = state.block_len(from.block).saturating_sub(from.offset) + to.offset;
    for block in from.block + 1..to.block {
        total += state.block_len(block) + 1;
    }
    total + 1
}
