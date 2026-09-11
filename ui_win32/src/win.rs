// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The window: its class, its procedure, and the loop that pumps it.
//!
//! **Windows only, and the only file in this crate that holds state.** Everything it decides is
//! decided somewhere portable — `sheet/geom.rs` for where a cell is, `sheet/draw.rs` for what it
//! looks like, `theme.rs` for what colour it is — so this file is a translator between Windows'
//! messages and those answers, plus the one piece of genuinely unsafe machinery the shell needs.
//!
//! ## The `GWLP_USERDATA` arrangement
//!
//! A window procedure is a C callback: it gets an `HWND` and no context. The standard Win32
//! answer, and the one used here, is to `Box` the state, hand the raw pointer to the window in
//! `GWLP_USERDATA`, and read it back on every message. The safety of that rests on three facts
//! and they are worth stating because a future edit can break any of them:
//!
//! 1. The pointer is stored in `WM_NCCREATE`, from the `CREATESTRUCTW` the caller passed —
//!    before any message that reads it can arrive.
//! 2. It is taken back and dropped in `WM_NCDESTROY`, the *last* message a window ever gets,
//!    and the slot is zeroed at the same time. Nothing reads it afterwards.
//! 3. **A message is never dispatched re-entrantly while the borrow is live.** Every handler
//!    below takes the `&mut` for the length of one message and returns. This is
//!    `doc/windows-shell.md`'s decision 7, and from W3 it has teeth: `dialog.rs`'s file
//!    dialogs, message boxes and text prompt each run a nested message loop, so every handler
//!    that opens one borrows the state **on each side of the call and never across it**.
//!    [`sheet_rename`] is the shape written out: one [`with_sheet`] to read what the prompt
//!    needs, the prompt, then a *fresh* [`with_sheet`] to apply the answer — each with the
//!    SAFETY comment that says which side of the nested loop it is on.
//!
//! ## What W3 added, and where the state for it lives
//!
//! Editing needs somewhere to put a keystroke that is not the document. Three things, and none
//! of them is a second model:
//!
//! * a **mode** ([`crate::sheet::state::Mode`]) — Ready, Enter or Edit, decided by a pure
//!   function and stored here because the *next* keystroke depends on it;
//! * a child `EDIT` holding the in-progress text, which is decision 2's line exactly — a
//!   control that holds a keystroke is a widget, a control that holds the document is a second
//!   model, and this one is emptied into `App::enter` and hidden again;
//! * a **dirty flag**, which the core deliberately does not have. It is set by an
//!   [`grind_core::Observer`] that posts [`WM_DOC_CHANGED`] rather than by each handler
//!   remembering to, so undo, redo and a recalculation mark the document modified without
//!   anybody wiring them up (architecture rule 3: the core pushes, shells never poll).

#![cfg(windows)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, ClientToScreen, EndPaint, HDC, InvalidateRect, OPAQUE, PAINTSTRUCT, SetBkColor,
    SetBkMode, SetTextColor, UpdateWindow,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForWindow, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CREATESTRUCTW, CS_DBLCLKS, CW_USEDEFAULT, CheckMenuItem, CreateMenu,
    CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow, DispatchMessageW,
    EN_CHANGE, EN_KILLFOCUS, ES_AUTOHSCROLL, GWLP_USERDATA, GetMessageW, GetParent,
    GetWindowLongPtrW, GetWindowTextLengthW, GetWindowTextW, HMENU, IDC_ARROW, LoadCursorW,
    MF_BYCOMMAND, MF_CHECKED, MF_POPUP, MF_SEPARATOR, MF_STRING, MF_UNCHECKED, MSG, MoveWindow,
    PostMessageW, PostQuitMessage, RegisterClassW, SB_BOTTOM, SB_HORZ, SB_LINEDOWN, SB_LINEUP,
    SB_PAGEDOWN, SB_PAGEUP, SB_THUMBPOSITION, SB_THUMBTRACK, SB_TOP, SB_VERT, SCROLLINFO,
    SCROLLINFO_MASK, SIF_PAGE, SIF_POS, SIF_RANGE, SPI_GETWHEELSCROLLLINES, SW_HIDE, SW_SHOW,
    SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOZORDER, SendMessageW, SetMenu, SetWindowLongPtrW,
    SetWindowPos, SetWindowTextW, ShowWindow, SystemParametersInfoW, TPM_RETURNCMD,
    TPM_RIGHTBUTTON, TrackPopupMenuEx, TranslateMessage, WHEEL_DELTA, WM_APP, WM_CHAR, WM_CLOSE,
    WM_COMMAND, WM_CONTEXTMENU, WM_CREATE, WM_CTLCOLOREDIT, WM_DESTROY, WM_DPICHANGED,
    WM_ERASEBKGND, WM_HSCROLL, WM_IME_STARTCOMPOSITION, WM_KEYDOWN, WM_KILLFOCUS, WM_LBUTTONDBLCLK,
    WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_NCCREATE, WM_NCDESTROY, WM_PAINT,
    WM_SETFOCUS, WM_SETFONT, WM_SETTINGCHANGE, WM_SIZE, WM_VSCROLL, WNDCLASSW, WS_CHILD,
    WS_HSCROLL, WS_OVERLAPPEDWINDOW, WS_VSCROLL,
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
use crate::metrics::{Faces, Fonts};
use crate::notice;
use crate::problems;
use crate::sheet::assist;
use crate::sheet::clip;
use crate::sheet::draw::{self, Frame};
use crate::sheet::geom::{GridGeom, Hit, MAX_COLS, MAX_ROWS, Rect, Sizes, scale};
use crate::sheet::keymap::{self, Dir, Selection};
use crate::sheet::state::{self, Outcome, Seed};
use crate::sheet::status;
use crate::surrogate;
use crate::text;
use crate::text::geom::{Flow, Page, StripHit};
use crate::theme::{self, Mode, Theme};
use grind_core::DocumentKind;
use grind_sheet::{App, Filter, Pos, RecalcMode, TableOptions};
use grind_text::{Caret, Layout, markdown};

/// The display name, which is not the file name (`doc/windows-shell.md`, decision 1).
const APP_NAME: &str = "Grind";
const CLASS_NAME: &str = "GrindWindowClass";

/// The face everything in this window is drawn in — **asked for rather than assumed**, since W10.
///
/// Windows 11's shell font is *Segoe UI Variable*, a variable font whose optical sizes GDI sees
/// as three families; `Text` is the one meant for 12 to 24 pixels, which is every size in this
/// shell's ramp. Windows 10 has none of them and Segoe UI is the shell font there. Asking for a
/// face GDI does not have does **not** fail — it substitutes something arbitrary — so the two are
/// tried in order against `EnumFontFamiliesExW`, which is what makes this a probe and not a
/// guess (`gdi::ui_face`). Under Wine neither is usually installed and the fallback is whatever
/// GDI substitutes, which is the path `doc/windows-shell.md` says every screenshot taken there
/// exercises.
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

/// "A key went to one of our child controls", sent by the message loop.
///
/// Enter, Escape, Tab and the arrows never reach a child `EDIT`'s parent on their own — the
/// control swallows them — so the pump in [`run`] relays them and lets the window decide. That
/// is the standard answer for a window with no dialog manager, and it is preferred here over
/// subclassing the control: a subclass means a second window procedure and a second lifetime to
/// get right, and `SetWindowSubclass` lives in `comctl32`, which this binary does not import
/// and would rather not start importing for one keystroke.
///
/// The reply is what makes it a *question* rather than a notification: non-zero for a key the
/// shell claimed, zero for one the control should go on and handle itself.
const WM_CHILD_KEY: u32 = WM_APP + 1;

/// "The document changed", posted by [`Changed`].
const WM_DOC_CHANGED: u32 = WM_APP + 2;

/// The frame `--render-to` draws, in pixels at 96 dpi.
///
/// Fixed rather than taken from the command line, because the output's whole purpose is to be
/// compared with another one: a size that could differ between two runs is a difference the
/// comparison would report as a drawing change.
const RENDER_W: i32 = 1280;
const RENDER_H: i32 = 800;

/// Everything the window owns. One per window, boxed, reached through `GWLP_USERDATA`.
///
/// **One binary, both document types** (decision 1), and this is where that stops being a claim
/// about `main.rs` and becomes one about the window: which pane a window *is* comes from
/// [`grind_core::kind()`] reading the file's bytes, and every message below either belongs to one
/// pane — [`with_sheet`], [`with_text`] — or is answered for both here.
///
/// The two arms deliberately keep their own path, theme and dirty flag rather than sharing a
/// header struct. That is not duplication for its own sake: it is what let the spreadsheet's
/// forty handlers keep the shape they already had, borrowing exactly the state they always
/// borrowed, when the second pane arrived. A pane is boxed because the two are very different
/// sizes and the enum would otherwise be as big as the larger.
enum Pane {
    Sheet(Box<Sheet>),
    Text(Box<Text>),
}

impl Pane {
    fn sheet_mut(&mut self) -> Option<&mut Sheet> {
        match self {
            Pane::Sheet(sheet) => Some(sheet),
            Pane::Text(_) => None,
        }
    }

    fn text_mut(&mut self) -> Option<&mut Text> {
        match self {
            Pane::Text(text) => Some(text),
            Pane::Sheet(_) => None,
        }
    }

    /// The pending `WM_CHAR` high surrogate, whichever pane this is — `typed_char`'s slot,
    /// reached through `Pane` rather than `with_sheet`/`with_text` because a supplementary-plane
    /// character can arrive whether the window is showing a grid or a document.
    fn surrogate_mut(&mut self) -> &mut Option<u16> {
        match self {
            Pane::Sheet(sheet) => &mut sheet.surrogate,
            Pane::Text(text) => &mut text.surrogate,
        }
    }

    fn theme(&self) -> Theme {
        match self {
            Pane::Sheet(sheet) => sheet.theme,
            Pane::Text(text) => text.theme,
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
        }
    }

    fn path(&self) -> Option<PathBuf> {
        match self {
            Pane::Sheet(sheet) => sheet.path.clone(),
            Pane::Text(text) => text.path.clone(),
        }
    }

    /// Which document kind this pane is showing, for the Save dialog's filters and suggested
    /// name — `dialog::save_path` needs to know whether to offer `.fods`/`.ods` or
    /// `.fodt`/`.odt`.
    fn kind(&self) -> DocumentKind {
        match self {
            Pane::Sheet(_) => DocumentKind::Spreadsheet,
            Pane::Text(_) => DocumentKind::Text,
        }
    }

    fn set_dirty(&mut self, dirty: bool) {
        match self {
            Pane::Sheet(sheet) => sheet.dirty = dirty,
            Pane::Text(text) => text.dirty = dirty,
        }
    }

    fn dirty(&self) -> bool {
        match self {
            Pane::Sheet(sheet) => sheet.dirty,
            Pane::Text(text) => text.dirty,
        }
    }

    /// What the document is called, for a title bar and for the close question.
    fn document_name(&self) -> String {
        let path = self.path();
        match &path {
            Some(path) => path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.display().to_string()),
            None => "Untitled".to_owned(),
        }
    }

    fn title(&self) -> String {
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
        };
        match result {
            Ok(()) => {
                match self {
                    Pane::Sheet(sheet) => sheet.path = Some(path.to_owned()),
                    Pane::Text(text) => text.path = Some(path.to_owned()),
                }
                self.set_dirty(false);
                Ok(())
            }
            Err(error) => Err(error),
        }
    }
}

/// The document's very first caret position. `Caret` has no `Default`, and spelling this out once
/// is better than spelling `{ block: 0, offset: 0 }` in five places.
const START: Caret = Caret {
    block: 0,
    offset: 0,
};

/// Everything the *spreadsheet* pane owns.
struct Sheet {
    app: grind_sheet::App,
    path: Option<PathBuf>,
    sheet: usize,
    geom: GridGeom,
    theme: Theme,
    /// An anchor and an active cell — presentation state, and the core is never told about it.
    /// A range reaches `App` as two positions when something is done to it, which in W2 is only
    /// the status bar's aggregates.
    selection: Selection,
    /// What a held mouse button is extending, if anything.
    drag: Option<Drag>,
    /// The child `EDIT` that *is* the name box while somebody is typing in it.
    ///
    /// Created hidden and shown over the drawn box on demand. The box itself is painted by
    /// `sheet/draw.rs` the rest of the time, which is what lets `--render-to` show an address
    /// with no control and no window anywhere.
    name_box: HWND,
    name_box_open: bool,
    /// The child `EDIT` that holds an edit in progress — the in-cell editor and the formula
    /// bar, which are **one control in two places** rather than two controls. Decision 2 is the
    /// line it sits on: a control that holds a keystroke is a widget, and a control that holds
    /// the document is a second model. This one is emptied into [`grind_sheet::App::enter`] and
    /// hidden again.
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
    /// What is being offered to somebody typing a formula, and which offer Tab would take —
    /// `sheet/assist.rs`, recomputed from the editor's text on every keystroke rather than
    /// updated in place.
    assist: assist::Assist,
    /// The assist band's runs, as [`assist::band`] renders whatever `assist` holds. Kept beside
    /// it rather than recomputed inside the paint, because it is the *emptiness* of this that
    /// decides `geom.hint_h` — a band with height and nothing in it is a gap, and one with
    /// something in it and no height draws over the headers. Always set through [`Sheet::hint`],
    /// which is what keeps the two agreeing, exactly as [`Sheet::say`] does for the notice bar.
    hint: Vec<assist::Piece>,
    /// Whether the formula bar shows the friendly *reading* of a formula rather than the text
    /// that would be typed back in — View ▸ Friendly Formulas. Presentation state, like the
    /// selection and the overlays: nothing here is ever written, and the document's formula stays
    /// exactly the one ODF spells (R1).
    friendly: bool,
    /// The face the two child `EDIT`s are set in. Owned here because a `WM_SETFONT` does not
    /// take a copy: the handle has to outlive every paint of the control, and be deleted after
    /// it.
    ui_font: Option<Font>,
    /// The ground the child `EDIT`s are painted on, answered to `WM_CTLCOLOREDIT`. Built on
    /// demand and thrown away when the theme changes, because a brush is a colour and the
    /// colour is the theme's.
    field_brush: Option<Brush>,
    /// A `WM_CHAR` high surrogate waiting for the low half that completes it — `typed_char`'s
    /// state, kept here rather than in a static because two windows must not share it. Only
    /// matters in Ready mode: once an edit is open, the native `EDIT` control has focus and
    /// assembles the pair itself, the way every Win32 control does.
    surrogate: Option<u16>,
    /// `doc/view-modes.md`'s two overlays, W6's — presentation state exactly like `selection`:
    /// never told to the core, and asked for fresh on every `get_viewport_with` rather than
    /// stored on a cell, which is what keeps opening one on every R7 document and saving again
    /// byte-identical (a stored classification goes stale; a derived one cannot).
    overlays: grind_sheet::view::Overlays,
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
    /// Rebuild the geometry for the current client size and DPI.
    ///
    /// Everything measured is rebuilt from the *document's* lengths rather than scaled from the
    /// last answer, so a window dragged between two monitors and back is pixel-identical to one
    /// that never moved — repeatedly scaling a scaled number is how that drifts.
    fn relayout(&mut self, width: f64, height: f64, dpi: u32) {
        // The one place the sheet index is checked, and it is here because *every* path that
        // changes the document ends in a relayout. Deleting a sheet, undoing an insertion and
        // opening a smaller document all leave the index pointing past the end, and a painter
        // that is handed one has nothing useful to do with it — W3 found this by renaming a
        // sheet, see `doc/windows-shell.md`.
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
            strip_h: scale(draw::STRIP_H, dpi),
            banner_h: banner_h(self.banner.as_deref(), dpi),
            hint_h: hint_h(&self.hint, dpi),
            header_w: scale(draw::HEADER_W, dpi),
            header_h: scale(draw::HEADER_H, dpi),
            status_h: scale(draw::STATUS_H, dpi),
            cols: Sizes::from_lengths(
                scale(draw::COL_W, dpi),
                MAX_COLS,
                &widths,
                &hidden_cols,
                dpi,
            ),
            rows: Sizes::from_lengths(
                scale(draw::ROW_H, dpi),
                MAX_ROWS,
                &heights,
                &hidden_rows,
                dpi,
            ),
            first_row: self.geom.first_row,
            first_col: self.geom.first_col,
            width,
            height,
            dpi,
        };
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
                let selection = self.selection;
                let (app, sheet) = (&self.app, self.sheet);
                let occupied = |pos: Pos| {
                    app.get(sheet, pos)
                        .is_ok_and(|value| !matches!(value, grind_sheet::model::CellValue::Empty))
                };
                let to = keymap::moved(selection, motion, extend, extent, &occupied);
                // A hidden track is drawn as gone, so a cursor may not stop on one — see
                // `keymap::onto_visible`, which is the rule and has the tests.
                self.selection = keymap::onto_visible(to, motion, &self.geom.rows, &self.geom.cols);
            }
            // Everything the sheet *uses*, with the active cell at A1 so that the view goes
            // home rather than to the far corner. An empty sheet selects the one cell it has.
            keymap::Action::SelectAll => {
                let (rows, cols) = self.app.used_extent(self.sheet).unwrap_or((0, 0));
                self.selection = Selection {
                    anchor: Pos::new(rows.saturating_sub(1), cols.saturating_sub(1)),
                    active: Pos::new(0, 0),
                };
            }
            keymap::Action::GoTo => {}
        }
        self.reveal();
    }

    /// Scroll the least it takes to put the active cell in the body, and no more.
    ///
    /// The whole of it is one clamp per axis, and the two ends say what the two cases are: the
    /// view may not start after the active cell (it would be above the top) and may not start
    /// before the least first track that shows it (it would be below the bottom). A cell already
    /// in view falls between the two and nothing moves.
    fn reveal(&mut self) {
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

    /// Put something in the notice bar, or take it away.
    ///
    /// One function because the sentence and the *height* have to change together: a banner
    /// with a sentence and no height draws nothing at all, and one with a height and no
    /// sentence is a bar of colour nobody can dismiss.
    fn say(&mut self, notice: Option<String>) {
        self.geom.banner_h = banner_h(notice.as_deref(), self.geom.dpi);
        self.banner = notice;
    }

    /// Recompute the assist band from what the editor holds, and set its height with it.
    ///
    /// The counterpart to [`Sheet::say`] and the same one-function rule: the runs and the height
    /// change together, so no path can leave a band with nothing in it or a run with nowhere to
    /// go. `caret` is a **byte** offset into `text` — [`state::byte_at`] is the conversion from
    /// what `EM_GETSEL` counts.
    ///
    /// Returns whether the band's height changed, which is the difference between repainting the
    /// strip and relaying out the window: the grid moves down when the band appears.
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

    /// Where the edit control goes.
    ///
    /// Over the active cell, unless that cell is scrolled out of sight or the edit began on the
    /// formula bar — in which case the bar is where the text is, and typing into a control
    /// nobody can see is the alternative.
    fn editor_rect(&self) -> Rect {
        let active = self.selection.active;
        let cell = self.geom.editor_rect(active.row, active.col);
        match self.editor_on_bar || cell.w <= 0.0 || cell.h <= 0.0 {
            true => self.geom.formula_rect(),
            false => cell,
        }
    }
}

/// Everything the *word processor's* pane owns — **W5**.
///
/// Deliberately smaller than [`Sheet`], and the difference is where the work went: a spreadsheet
/// pane holds a grid geometry it can compute from four numbers, and a text pane holds a
/// [`text::geom::Flow`] that had to be *measured*. Which is why the font cache is here and not in
/// `gdi.rs`: it is state, it is expensive, and it belongs to the pane that measures with it.
struct Text {
    app: grind_text::App,
    path: Option<PathBuf>,
    theme: Theme,
    dirty: bool,
    /// The window's bands and how far down the document the body starts.
    page: Page,
    /// Where every block sits, measured. Rebuilt whenever the document, the width or the DPI
    /// changes and **not** per paint — a paint re-lays-out only the blocks it can see.
    flow: Flow,
    /// The caret and the other end of the selection. Presentation state: the core is never told
    /// about it, exactly as the grid's `Selection` is never told to `grind_sheet::App`.
    caret: Caret,
    anchor: Caret,
    /// The column Down and Up aim for, in pixels, kept across a run of vertical keystrokes and
    /// dropped the moment the caret moves horizontally. A property of a *run of keystrokes*
    /// rather than of the document, which is why `App::caret_line` takes it rather than storing
    /// it.
    goal_x: Option<f32>,
    /// The fonts everything is measured and drawn with (`metrics.rs`). `None` only before the
    /// first `WM_CREATE`, and rebuilt whole on `WM_DPICHANGED` — a font is a size in *pixels*,
    /// so every one of them is wrong on a monitor with different scaling.
    fonts: Option<Fonts>,
    /// The DPI `fonts` was built for, which is the only reason to build another.
    fonts_dpi: u32,
    /// Whether a drag is extending the selection.
    dragging: bool,
    /// Whether a caret should be drawn at all, in the one place that still draws one: `render`'s
    /// windowless frame, which has no `HWND` and so no system caret. In a real window the system
    /// caret (`place_system_caret`) is the caret and this field is left `true` and ignored —
    /// still set after every edit and motion, on the off chance a caller draws a frame from this
    /// state with no window, which is exactly what `render` does.
    caret_on: bool,
    banner: Option<String>,
    /// What [`grind_text::App::type_markdown`] said the next character must be set in — the
    /// style a completed `**bold**` span leaves behind, carried across keystrokes so the shell
    /// does not need its own idea of the notation. `ui_tui`'s `resume` is the same field.
    resume: Option<grind_text::CharStyle>,
    /// A `WM_CHAR` high surrogate waiting for its low half — `Sheet::surrogate`'s twin, and the
    /// one that matters every keystroke rather than only at the start of an edit: this pane has
    /// no native control to assemble a pair for it.
    surrogate: Option<u16>,
    /// `doc/view-modes.md`'s name overlay, this pane's own — `:names`' equivalent, a bookmark
    /// drawn beside the text it anchors. Presentation state, exactly like `Sheet::overlays`.
    show_names: bool,
    /// Which format-strip control is held down, and therefore the one a release over it will
    /// activate. See `text_button_down`.
    pressed: Option<text::geom::StripHit>,
    /// Which format-strip control the pointer is over, or `None` when it is anywhere else (W10).
    ///
    /// The only state in this shell that exists purely so that something *looks* alive, and it
    /// earns it: a row of drawn buttons with no hover feedback is the single thing that most
    /// gives a custom-painted window away. Repainting is guarded on the value actually changing,
    /// so an ordinary mouse move across the document costs one comparison.
    hover: Option<text::geom::StripHit>,
}

impl Text {
    /// Rebuild the bands, the fonts if the monitor changed, and the flow.
    ///
    /// Everything measured is rebuilt from the document rather than scaled from the last answer,
    /// which is the same rule [`Sheet::relayout`] follows and for the same reason: repeatedly
    /// scaling a scaled number is how a window dragged between two monitors and back stops being
    /// pixel-identical to one that never moved.
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
    /// ponytail: this lays out the **whole document**, so a five-hundred-page one is measured on
    /// every resize. It is what makes the scrollbar honest — a flow that only knew about the
    /// blocks on screen could not say how tall the document is — and it is fast enough for
    /// everything in `text/tests/data/`. The upgrade when it stops being is the usual one: an
    /// estimated height for blocks nobody has looked at, replaced by the real one as they scroll
    /// past. The trigger is a document where a resize is visibly slow, and not before.
    fn reflow(&mut self) {
        let Some(faces) = self.faces() else {
            return;
        };
        let dpi = self.page.dpi;
        // A picture is measured from its own decoded pixels rather than as a line of text —
        // `flow_of`'s own doc comment is the rule, and this closure is `image.rs`'s answer to
        // it: fit the column, keep the aspect ratio, never larger than the picture's own size,
        // the same rule `ui_text_gtk`'s `image_size` follows. `None` when the block is not a
        // picture, or WIC could not read it (a corrupt file, a format nobody's decoder knows) —
        // R5's tolerance, so a bad picture takes no more room than its placeholder character
        // would rather than stopping the document.
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
        self.flow = text::geom::flow_of(&self.app, &faces, dpi, &picture);
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
        self.app
            .get_viewport(index..index + 1)
            .get(index)
            .map(|view| view.text.chars().count())
            .unwrap_or(0)
    }

    fn last_caret(&self) -> Caret {
        let block = self.app.block_count().saturating_sub(1);
        Caret {
            block,
            offset: self.block_len(block),
        }
    }

    /// The faces this document is set in, at the current measure. Built per call rather than
    /// kept, because it borrows the font cache and a `&Faces` living in the state would borrow it
    /// for the life of the window; the fonts themselves are cached, so this costs a handful of
    /// map lookups.
    fn faces(&self) -> Option<Faces<'_>> {
        let fonts = self.fonts.as_ref()?;
        let (_, width) = self.page.text_column();
        Some(Faces::new(
            fonts,
            width,
            scale(text::geom::INDENT, self.page.dpi),
        ))
    }

    /// One block's lines, measured in that block's own face.
    ///
    /// A value, computed and thrown away — the same contract `App::get_viewport` offers for
    /// content. Nothing here keeps a layout, because a kept one goes stale on the next keystroke
    /// and there is no cheap way to know it has.
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

    /// Where the caret is drawn, in client pixels, and how big it is — the same geometry
    /// `text/draw.rs` uses to paint it, answered without a `Frame` because neither caller here
    /// has a reason to build one. `position_ime_composition` wants the point: a composing IME's
    /// candidate list should appear where the caret is rather than wherever Windows last happened
    /// to leave it. `place_system_caret` wants all four, because `CreateCaret` takes a size and
    /// the caret is a different height on a heading than on a paragraph.
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

    /// Where a motion lands, without moving anything.
    ///
    /// Every vertical answer is [`grind_text::App`]'s — `caret_line` and `caret_line_bounds`,
    /// measured through this pane's own [`Faces`] — because `doc/text-layout.md` put line layout
    /// in the core precisely so that four shells could not answer Down-arrow four ways. What is
    /// left here is the horizontal half, which is about characters and not about lines.
    fn moved(&self, motion: text::keymap::Motion, goal: f32) -> Caret {
        use text::keymap::Motion;
        let at = self.caret;
        match motion {
            Motion::Char(step) if step < 0 => match at.offset {
                0 if at.block > 0 => Caret {
                    block: at.block - 1,
                    offset: self.block_len(at.block - 1),
                },
                0 => at,
                offset => Caret {
                    block: at.block,
                    offset: offset - 1,
                },
            },
            Motion::Char(_) => match at.offset < self.block_len(at.block) {
                true => Caret {
                    block: at.block,
                    offset: at.offset + 1,
                },
                // Off the end of a block is the start of the next one: a document is one flow,
                // not a list of boxes, which is the same rule `App::caret_line` follows.
                false if at.block + 1 < self.app.block_count() => Caret {
                    block: at.block + 1,
                    offset: 0,
                },
                false => at,
            },
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

    /// Move the caret, extending the selection or collapsing it.
    ///
    /// The **goal column** is remembered across a run of vertical keystrokes and dropped the
    /// moment the caret moves horizontally, which is what makes walking Down through a short line
    /// and out the other side come back to the column it started in.
    fn navigate(&mut self, motion: text::keymap::Motion, extend: bool) {
        let vertical = matches!(motion, text::keymap::Motion::Line(_));
        let goal = match vertical {
            true => self.goal_x.or_else(|| self.caret_x()),
            false => None,
        };
        self.caret = self.moved(motion, goal.unwrap_or(0.0));
        if !extend {
            self.anchor = self.caret;
        }
        self.goal_x = goal;
        self.reveal();
    }

    /// Put the caret somewhere and forget the goal column and the selection — what a click does,
    /// and what every edit does with the caret it leaves behind.
    fn place(&mut self, at: Caret, extend: bool) {
        self.caret = at;
        if !extend {
            self.anchor = at;
        }
        self.goal_x = None;
        self.reveal();
    }

    /// Which caret position a point in the window is nearest.
    ///
    /// **Nearest, never nothing** — [`Flow::at_y`]'s rule, carried through to the offset:
    /// a click in the margin of a line lands at that line's near end, because a document has no
    /// "outside" and a caret that refuses to move is a bug nobody can see the cause of.
    fn caret_at(&self, x: f64, y: f64) -> Option<Caret> {
        let body = self.page.body();
        let document_y = y - body.y + self.page.scroll;
        let block = self.flow.at_y(document_y)?;
        let slot = self.flow.slot(block).copied()?;
        let layout = self.layout_of(block)?;
        let (column_x, _) = self.page.text_column();
        let local_y = (document_y - slot.top).max(0.0);
        let line = layout
            .lines()
            .iter()
            .position(|line| local_y < f64::from(line.top + line.height))
            .unwrap_or(layout.lines().len().saturating_sub(1));
        let local_x = (x - column_x - slot.indent) as f32;
        Some(Caret {
            block,
            offset: layout.offset_at(line, local_x),
        })
    }

    /// How many lines a Page Up or Page Down moves.
    ///
    /// Measured in *body* lines rather than in pixels, and one short of a screenful, which is
    /// what keeps a line of context across the jump — the same rule the grid's page has.
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

/// The bridge from the core to the window: *something changed*.
///
/// Architecture rule 3 — the core pushes and shells never poll — reaching a message queue. It
/// **posts** rather than sends, and that is not an optimisation: `App::mutate` notifies with its
/// write lock dropped but still inside the call that made the change, and a `SendMessageW` there
/// would re-enter this window's procedure while a handler is holding `&mut State`. Posting puts
/// the notification in the queue, where it is handled long after the borrow is gone.
///
/// The window is held as an `isize` rather than as an `HWND` so that this is `Send + Sync`
/// without an unsafe promise: [`grind_core::Observer`] requires both, because the core does not
/// say which thread a change arrives on.
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

/// A document, read, with a window's worth of state around it and no window.
///
/// Shared by [`run`] and [`render`] on purpose: the render path is a *second caller* of every
/// answer the window uses, not a second set of answers, which is what makes a byte-identical
/// frame evidence about the real thing (`doc/windows-shell.md`, decision 5).
///
/// **Which pane comes out is decided by the caller**, from `grind_core::kind` reading the file's
/// bytes — never from its name, because a spreadsheet does not become a document by being called
/// one. This function only obeys.
fn opened(kind: DocumentKind, path: Option<PathBuf>, theme: Theme) -> Result<Pane, String> {
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

/// The word processor's pane, on a document or on nothing.
fn opened_text(path: Option<PathBuf>, theme: Theme) -> Result<Text, String> {
    let app = grind_text::App::new();
    if let Some(path) = &path {
        app.open_file(path)
            .map_err(|error| format!("{}: {error}", path.display()))?;
    }
    Ok(Text {
        app,
        path,
        theme,
        dirty: false,
        page: Page {
            status_h: text::geom::STATUS_H,
            dpi: 96,
            ..Page::default()
        },
        flow: Flow::default(),
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
        hover: None,
        pressed: None,
    })
}

fn opened_sheet(path: Option<PathBuf>, theme: Theme) -> Result<Sheet, String> {
    let app = grind_sheet::App::new();
    if let Some(path) = &path {
        app.open_file(path)
            .map_err(|error| format!("{}: {error}", path.display()))?;
    }
    Ok(Sheet {
        app,
        path,
        sheet: 0,
        geom: GridGeom {
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
        },
        theme,
        selection: Selection::default(),
        drag: None,
        name_box: HWND::default(),
        name_box_open: false,
        editor: HWND::default(),
        editor_on_bar: false,
        mode: state::Mode::default(),
        dirty: false,
        banner: None,
        assist: assist::Assist::default(),
        hint: Vec::new(),
        // **On**, which is `ui_sheet_gtk`'s own default (`chrome::formula_bar(.., true)`) and is
        // the reason to have one default rather than two: a formula reads the same in both
        // windows unless somebody says otherwise. It costs nothing to be wrong about, either —
        // the reading is only ever *shown*, and the moment the cell is opened for editing the
        // bar is back to the text that will be stored.
        friendly: true,
        ui_font: None,
        field_brush: None,
        surrogate: None,
        overlays: grind_sheet::view::Overlays::NONE,
    })
}

/// Draw one frame of a document to a `.bmp` and exit — **with no window, no compositor and no
/// display** (`doc/windows-shell.md`, decision 5).
///
/// Not a user feature. It is how custom drawing gets an assertable output: a refactor is proved
/// one when the file comes back byte-identical. Everything that could differ between two runs is
/// pinned rather than read — the size is a constant, and the **theme is forced to light**,
/// because a screenshot compared against another one must not depend on what the machine
/// running it has under `Themes\Personalize`.
pub fn render(
    kind: DocumentKind,
    path: Option<PathBuf>,
    target: &std::path::Path,
    dark: bool,
) -> Result<(), String> {
    // COM, for `image.rs`'s WIC calls — a picture is measured in `relayout` and drawn in
    // `draw_text_frame`, and this windowless path has no `run`'s own guard to lean on.
    let _com = Com::new();
    // The palette this frame is drawn in is the **flag's**, never the machine's: two renders of
    // one document on two machines have to be the same bytes, and a registry read would make the
    // developer's own theme part of the output. `--dark` is how the dark palette gets looked at
    // at all without a Windows machine set to dark (W10) — before it, half of `theme.rs` shipped
    // unseen.
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
    }
    std::fs::write(target, dib.bmp()).map_err(|error| format!("{}: {error}", target.display()))
}

/// Open a window on a document and pump messages until it closes.
///
/// `path` is `None` for a new, empty document of `kind` — **both kinds**, since W5: one binary,
/// one window class, and the pane decided by the bytes.
pub fn run(kind: DocumentKind, path: Option<PathBuf>) -> Result<(), String> {
    // Before *any* window exists, which is the whole requirement: per-monitor v2 cannot be set
    // once a window has been created, and asking for it late fails silently and leaves the
    // process system-DPI-aware — a window that is bitmap-stretched and blurry on a 150% monitor.
    // Best-effort because it also fails when a manifest already set an awareness, and that is
    // not a reason to refuse to start.
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
            // Enter, Escape, Tab and the arrows inside a child `EDIT` never reach its parent:
            // the control eats them, and with no dialog manager there is nothing to turn them
            // into a command. Relaying them here is the standard answer, and it is cheaper and
            // shorter-lived than subclassing the control — see `WM_CHILD_KEY`.
            //
            // The pump asks rather than decides, and reaches no state at all: "the message went
            // to a child of ours" is everything it knows, and the window answers whether the
            // key was claimed.
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
                    // at the call site — `dialog::use_theme` says why.
                    dialog::use_theme(theme);
                    // Registered here rather than in `opened`, because it needs a window to
                    // post to — and because a document read *before* there is one must not
                    // arrive as a change the user made. That is the whole of why this shell
                    // needs no "loading" flag: nobody is listening yet.
                    let observer = Arc::new(Changed(hwnd.0 as isize));
                    match pane {
                        Pane::Sheet(sheet) => sheet.app.set_observer(observer),
                        Pane::Text(text) => text.app.set_observer(observer),
                    }
                });
            }
            build_menu(hwnd);
            make_children(hwnd);
            refresh(hwnd);
            LRESULT(0)
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
        // A right click, **and** Shift+F10 or the keyboard's own Menu key — Windows sends this
        // one message for all three, which is what lets W7's context menus answer the keyboard
        // for free. `lparam`'s point is in *screen* coordinates here, unlike every mouse message
        // above, and is `(-1, -1)` exactly when the keyboard asked rather than the mouse.
        WM_CONTEXTMENU => {
            context_menu(hwnd, lparam);
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
        // A composing IME is about to show its own candidate window — moved to the caret before
        // `DefWindowProcW` draws it, so it appears where the text is going rather than wherever
        // Windows last happened to leave it. Everything else about composing (the candidate
        // list itself, and the committed result arriving as ordinary `WM_CHAR`s) is left to the
        // default handling; this is the one message this shell answers at all.
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
        // The X, Alt+F4 and the Exit item all arrive here, which is what makes them one verb —
        // and this is the last point at which "cancel" is still an answer.
        WM_CLOSE => {
            close(hwnd);
            LRESULT(0)
        }
        WM_MOUSEWHEEL => {
            wheel(hwnd, wparam);
            LRESULT(0)
        }
        // The system caret exists only while this window has keyboard focus — `CreateCaret` is
        // per-thread, so a window that never had focus never had one, and a dialog stealing
        // focus (or another application entirely) must not leave one behind for the next thing
        // Windows shows to trip over.
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
        // a great many settings, so the answer is to re-read rather than to trust the message:
        // reading a registry value is cheap and getting the condition wrong is a window stuck
        // in the wrong palette.
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
                    // at the call site — `dialog::use_theme` says why.
                    dialog::use_theme(theme);
                });
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
        // The last message this window will ever receive, which is what makes it the place to
        // take the box back. The slot is zeroed first, so a message arriving from within the
        // drop — there are none, but the ordering costs nothing — finds no state rather than a
        // dangling pointer.
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

/// Recompute everything derived from the document and the client area, then repaint.
///
/// One function rather than four, because the things it does are not independent: a different
/// sheet has different tracks, different tracks move the active cell, a moved active cell moves
/// the child control sitting on it, and every one of those changes what the frame looks like and
/// what the title says. Every path that touches the document ends here.
fn refresh(hwnd: HWND) {
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
    // The title is the *window's* rather than the pane's, so it is asked for separately — one
    // more borrow, taken and released like every other.
    // SAFETY: nothing inside dispatches.
    let title = unsafe { with_pane(hwnd, |pane| pane.title()) };
    let (Some(child), Some(title)) = (after, title) else {
        return;
    };
    // SAFETY: the borrow above is released. `MoveWindow` on a visible child repaints it and
    // `SetWindowTextW` repaints the caption — both dispatch messages, and therefore belong out
    // here. The title is compared first because this runs on every `WM_SIZE`, and rewriting the
    // caption on each pixel of a resize drag makes it flicker.
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

/// Put a child control on one of the geometry's rectangles.
///
/// The one place `Rect`'s floats become the integers Win32 wants, so a control cannot end up
/// half a pixel away from the cell it is editing.
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

/// The menu bar, built from [`menu::MENUS`] — `doc/windows-shell.md`'s decision 4.
///
/// A table rather than a sequence of calls, which buys two things: a command's id is derived
/// from its position and never written down twice, so the classic Win32 bug of two items sharing
/// a `WM_COMMAND` id cannot happen; and the check that every command is reachable from a menu
/// runs on Linux with no window at all.
///
/// Every item the current pane has no answer for — `menu::applies_to`'s question — is **left out
/// of the bar**, via `menu::items_for`/`menu::menu_has_items`, rather than shown and greyed. A
/// grey item was tried first (W5b's "a menu that knows which pane it is over") and did not
/// survive contact with the actual shape of the two panes' verbs: the sheet's own six so
/// outnumber the universal ones that `&Sheet` and half of `&View` stayed on screen, greyed, on
/// every document that was not a spreadsheet, and the bar read as a grid that had not noticed it
/// was showing a document. Omitting instead means a menu can end up with nothing left in it at
/// all — `&Sheet`/`&Data` on the text pane, `&Format` on the grid — which is exactly what
/// `menu_has_items` checks before a menu is put in the bar. This is rebuilt whenever the pane
/// itself changes (`adopt`) rather than the selection inside it, same as before; `Command::id`
/// still calls `do_command` for a command with no *item* on screen if one somehow arrives (a
/// stale accelerator, say), and `do_command`'s own per-pane no-ops are what makes that safe.
fn build_menu(hwnd: HWND) {
    // SAFETY: one borrow, for the kind and the two overlay checkmarks; nothing inside dispatches.
    // The role overlay has no meaning on the text pane (`CellRole` is the grid's alone), so it
    // reads `false` there rather than a second flag nothing ever sets.
    let (kind, roles_on, names_on, friendly_on) = unsafe {
        with_pane(hwnd, |pane| match pane {
            Pane::Sheet(sheet) => (
                DocumentKind::Spreadsheet,
                sheet.overlays.roles,
                sheet.overlays.names,
                sheet.friendly,
            ),
            Pane::Text(text) => (DocumentKind::Text, false, text.show_names, false),
        })
    }
    .unwrap_or((DocumentKind::Spreadsheet, false, false, false));
    // SAFETY: every label buffer outlives the `AppendMenuW` that reads it — Windows copies the
    // string — and the bar belongs to the window from `SetMenu` until it is destroyed with it.
    unsafe {
        let Ok(bar) = CreateMenu() else { return };
        for menu in menu::MENUS {
            // A menu with nothing this pane answers to — `Sheet`/`Data` on the text pane,
            // `Format` on the grid — is left out of the bar entirely rather than added with an
            // empty popup under its title.
            if !menu::menu_has_items(menu, kind) {
                continue;
            }
            let Ok(popup) = CreatePopupMenu() else {
                continue;
            };
            for item in menu::items_for(menu, kind) {
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
                        // The two overlays and the friendly bar are the checkable items this bar
                        // has — everything else is a plain verb with nothing to report back.
                        // Each is read once above rather than per item, the same "asked for
                        // fresh, never stored" shape the overlays themselves follow.
                        let checked = match command {
                            Command::ToggleRoles => Some(roles_on),
                            Command::ToggleNames => Some(names_on),
                            Command::ToggleFriendly => Some(friendly_on),
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
        }
        let _ = SetMenu(hwnd, Some(bar));
    }
}

/// W7's context menus: cells and headers get the grid's own clipboard verbs, the text pane its
/// own plus the five toggles the format strip already draws. A short, fixed list rather than a
/// state-aware one — the same simplification W7 already names for the menu bar's own greying —
/// and every entry is a real [`Command`] reused from [`menu::MENUS`] rather than a second
/// vocabulary, so a click here reaches exactly the handler a click on the bar would.
fn context_menu(hwnd: HWND, lparam: LPARAM) {
    let commands: &[Command] = match is_text(hwnd) {
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
        ],
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
    // SAFETY: the popup is built and destroyed within this call, and `TrackPopupMenuEx` is the
    // one nested message loop in it — decision 7's rule, and nothing is borrowed across it.
    let picked = unsafe {
        let Ok(popup) = CreatePopupMenu() else {
            return;
        };
        for command in commands {
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
/// coordinates. Simpler than anchoring it on the caret or the active cell, and it costs nothing
/// real — the menu still opens and still reaches every verb on it, which is the whole of what
/// Shift+F10 promises.
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

/// Create the two child `EDIT`s and the face they are both set in.
///
/// Hidden from birth and shown on demand, which is what lets the strip be *drawn* the rest of
/// the time and therefore appear in a `--render-to` frame that has no window at all. Decision 2
/// draws the line they sit on: a control that holds a keystroke is a widget, and one that holds
/// the document is a second model — these hold an address on its way to `status::locate` and a
/// cell's text on its way to `App::enter`, and nothing else.
fn make_children(hwnd: HWND) {
    // The text pane has no child controls at all — it draws its own caret and holds its own
    // keystrokes — and a window showing one therefore creates none. A window that *becomes* a
    // spreadsheet (File ▸ Open on a `.fods`) comes back through here, which is why this is
    // idempotent rather than once-only.
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

/// Tell Windows where the thumbs are and how big they are.
///
/// The range is in **tracks, not pixels** — see `GridGeom`'s comment: a `SCROLLINFO` is `i32`
/// and this sheet is twenty million pixels tall, but only a million rows.
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

/// The wheel, honouring the user's own setting rather than a constant.
///
/// `SPI_GETWHEELSCROLLLINES` is what the mouse control panel writes, and a user who set it to
/// one line or to a screenful means it. `WHEEL_DELTA` is one notch; a precision wheel sends
/// fractions of one, so the remainder would need carrying — a named simplification for W1,
/// where every notch is at least one row.
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
    // Zero means "do not scroll", which is a real setting. `WHEEL_PAGESCROLL` (0xFFFFFFFF)
    // means a screenful, and is answered with the page rather than with 4 294 967 295 rows.
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

/// A drag in progress, following the pointer.
///
/// Nothing happens when no button is down: a plain mouse move over a grid has to cost nothing,
/// because it arrives on every pixel of travel.
fn mouse_move(hwnd: HWND, lparam: LPARAM) {
    let (x, y) = point(lparam);
    if is_text(hwnd) {
        text_mouse_move(hwnd, x, y);
        return;
    }
    // SAFETY: no nested loop inside.
    let moved = unsafe {
        with_sheet(hwnd, |state| {
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

/// Where a mouse message happened, in client space.
///
/// The position is packed as two *signed* 16-bit numbers, and the sign matters: dragging above
/// or left of the client area gives a negative coordinate rather than a huge one.
fn point(lparam: LPARAM) -> (f64, f64) {
    (
        f64::from((lparam.0 & 0xffff) as i16),
        f64::from(((lparam.0 >> 16) & 0xffff) as i16),
    )
}

/// Which modifier keys are down, asked of the keyboard rather than carried by the message.
///
/// `WM_KEYDOWN` reports the key and nothing else, so this is where Ctrl+Down becomes different
/// from Down. `GetKeyState`'s high bit is "down"; its low bit is the *toggle* state, which is
/// what makes Caps Lock look pressed to anybody who tests for non-zero.
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

/// One keystroke from the window itself. `false` hands it back to `DefWindowProc`, which is
/// what leaves Alt+F4 and the system menu working.
fn key_down(hwnd: HWND, vk: u32) -> bool {
    if is_text(hwnd) {
        return text_key(hwnd, vk);
    }
    // SAFETY: no nested loop inside.
    let mode = unsafe { with_sheet(hwnd, |state| state.mode) }.unwrap_or_default();
    on_key(hwnd, mode, vk)
}

/// One keystroke that went to a child control instead, relayed by the pump. `true` means the
/// shell claimed it and the control must not see it.
///
/// The two children answer differently and that is the whole reason this asks which one it was:
/// the name box has two keys and no mode, and the editor has [`state::on_key`]'s whole table.
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
        // The offer list is asked **before** the editing state machine, because the three keys it
        // claims all mean something else there: Tab commits and moves right, Up and Down commit
        // in Enter mode, and Escape throws the whole edit away. `assist::on_key` answers `None`
        // unless a list is actually up, so with nothing offered this costs one match and every
        // key means exactly what it always meant.
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

/// Put the highlighted offer into the editor, replacing the word that was being typed.
///
/// `EM_REPLACESEL` rather than rewriting the whole control: it leaves the caret after what it
/// inserted — which for `SUM(` is exactly where the first argument goes — and it is one undo step
/// in the control's own history, so Ctrl+Z inside a half-typed formula still undoes a completion
/// rather than the whole edit.
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
    match state::on_key(mode, keymap::key_for(vk), mods()) {
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
            // SAFETY: no nested loop inside, and nothing on screen changes — F2 changes what
            // the *next* arrow key means and nothing else.
            unsafe { with_sheet(hwnd, |state| state.mode = state.mode.toggled()) };
            true
        }
    }
}

/// A character, after the keyboard layout and after the IME. `true` means it started an edit or
/// was otherwise claimed.
///
/// `WM_CHAR` carries one UTF-16 code unit at a time, so a character outside the Basic
/// Multilingual Plane — an emoji, most of the rarer CJK ideographs, some IME output — arrives as
/// two consecutive messages, and `char::from_u32` refuses each alone. `surrogate::combine` is
/// where the halves are reassembled; [`Pane::surrogate_mut`] is where the first one waits, which
/// is the pane's own state rather than a static because two windows must not share it, and per
/// pane rather than shared between them because a grid and a document mean two different open
/// documents once `adopt` has run.
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

/// Move a composing IME's own composition window to the caret.
///
/// Only the text pane needs this: the grid's editing happens inside a native `EDIT` control once
/// an edit is open, and every Win32 `EDIT` positions its own IME without being asked. This pane
/// draws its own caret and owns no control for Windows to ask, so with no help `CFS_DEFAULT`
/// leaves the composition window wherever it last was — often the top-left corner the first time
/// this process composes anything.
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
    if is_text(hwnd) {
        text_button_down(hwnd, lparam);
        return;
    }
    // An edit in progress is committed by clicking somewhere else, which is what every
    // spreadsheet does — and it has to happen **before** the selection moves, or the cell being
    // typed into would turn out to be the one that was just clicked.
    commit_edit(hwnd, None);
    let (x, y) = point(lparam);
    let extend = mods().shift;
    // The two fields on the strip are drawn chrome until they are clicked, at which point the
    // control hiding behind the drawing appears over it. Deciding that needs the geometry, so it
    // is read here and acted on below the borrow.
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
    if is_text(hwnd) {
        text_double_click(hwnd, lparam);
        return;
    }
    let (x, y) = point(lparam);
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
    if is_text(hwnd) {
        // A strip control that was pressed and is still under the pointer is *now* activated —
        // see `text_button_down`. Released before the verb runs, because three of the six open a
        // modal dialog and a borrow may not be held across one (decision 7).
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
    // SAFETY: no nested loop inside.
    unsafe {
        with_sheet(hwnd, |state| state.drag = None);
        let _ = ReleaseCapture();
    }
}

/// Put the caret in the name box: move the control over the drawn box, fill it with what the
/// box says, select all of it, and show it.
///
/// Every window call here happens with **nothing borrowed** — `ShowWindow` and `SetFocus` both
/// dispatch messages synchronously, so a borrow held across them is decision 7's aliasing bug in
/// its cheapest form.
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

/// Finish with the name box: go where it says, or put it away and go nowhere.
///
/// Idempotent, and that is load-bearing rather than defensive: hiding the control fires
/// `EN_KILLFOCUS`, which calls straight back in here. The flag is cleared *first*, so the second
/// call returns immediately instead of hiding a control that is already hidden and stealing the
/// focus back off whatever has just been given it.
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
            // Nonsense goes nowhere and says so by doing nothing: the box closes and the
            // selection stays where it was, which is less alarming than an error dialog for a
            // typo. `grind sheet view` is the R9 answer for a name this build cannot resolve.
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
    // What the seeded text already says, before a key is pressed: a cell holding `=SUM(B2:B4)`
    // opened with F2 shows that call's signature at once, and the function list's `=AVERAGE(`
    // shows what its first argument is.
    //
    // **Last**, and that ordering is the whole of it: `SetWindowTextW` fires `EN_CHANGE`, which
    // is wired to [`editor_changed`], which reads `EM_GETSEL` — and at that moment the control
    // has the new text with its caret still at **zero**, so the band it computes is the band for
    // an empty prefix, which is no band at all. Seeding it before the control was filled put a
    // signature up and had it wiped one message later; asking again here, after the caret is
    // where it belongs, is what makes it stay. `editor_changed` also refreshes, which is what
    // moves this control down when the band it just added took height out of the grid.
    editor_changed(hwnd);
}

/// Store what the editor holds and close it, moving the cursor if a key asked to.
///
/// Idempotent for [`close_name_box`]'s reason and by the same means: hiding the control fires
/// `EN_KILLFOCUS`, which is wired to call straight back in here.
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
    let store = match state::to_store(&text) {
        Ok(store) => store,
        // A formula that will not parse **does not commit**: the edit stays open with the caret
        // on the problem, because silently storing `=SUM(B2` as a piece of text is how a
        // spreadsheet loses somebody's work.
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

/// The text in the editor changed, so the drawn formula bar — which mirrors an in-cell edit —
/// and the assist band are recomputed and repainted.
///
/// **Only the two bands**, when the assist band was already the height it now wants: a keystroke
/// must not redraw the grid under it. When the band appeared or disappeared the grid really did
/// move — every rectangle below it is offset by `hint_h` — and then this is a full [`refresh`],
/// which is also what moves the editor control down with it.
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

/// Where the caret is in the editor, as a **byte** offset into `text`.
///
/// `EM_GETSEL` reports UTF-16 units and reports a *selection*; its end is the caret, which is
/// where typing would go and therefore what a completion is about. [`state::byte_at`] is the
/// conversion, and it is a pure function tested on Linux rather than arithmetic done here.
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

/// Run a verb, whether it arrived from a menu item or from its accelerator.
///
/// Every one of these acts on the **document**, so an edit in progress is committed first: the
/// half-typed cell is part of what Save would write and part of what Undo would take back.
///
/// The match is exhaustive on purpose. That is the other half of `menu.rs`'s reachability check
/// — a command with no handler fails the build, where a command in no menu fails a test — and it
/// is why neither of them needs a registry of ids.
fn do_command(hwnd: HWND, command: Command) {
    if is_text(hwnd) {
        text_command(hwnd, command);
        return;
    }
    // The one verb that means something *into* an open edit: picking a function while typing
    // inserts the call at the caret, where committing first would store whatever half-formula is
    // in the control — and a half-formula does not commit at all, it puts up a notice and keeps
    // the editor open, which is a poor answer to "which function did you want?".
    if matches!(command, Command::FunctionList) {
        function_list(hwnd);
        return;
    }
    commit_edit(hwnd, None);
    match command {
        Command::New => new_document(hwnd),
        Command::Open => open_document(hwnd),
        Command::Save => {
            save(hwnd);
        }
        Command::SaveAs => {
            save_as(hwnd);
        }
        // Not `DestroyWindow`: going through `WM_CLOSE` is what makes this item, Alt+F4 and the
        // title bar's X one verb with one close question.
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
        Command::GoTo => open_name_box(hwnd),
        Command::Recalculate => recalculate(hwnd),
        Command::ToggleFilter => toggle_filter(hwnd),
        Command::FormatTable => format_table(hwnd),
        Command::SheetAdd => sheet_add(hwnd),
        Command::SheetRename => sheet_rename(hwnd),
        Command::SheetDelete => sheet_delete(hwnd),
        Command::SheetNext => sheet_step(hwnd, 1),
        Command::SheetPrevious => sheet_step(hwnd, -1),
        Command::ShowSource => show_source(hwnd),
        Command::CheckDocument => check_document(hwnd),
        Command::ToggleRoles => toggle_overlay(hwnd, false),
        Command::ToggleNames => toggle_overlay(hwnd, true),
        // Handled above, before the commit — but the match stays exhaustive, which is what says
        // every command has a handler.
        Command::FunctionList => function_list(hwnd),
        Command::ExplainFormula => explain_formula(hwnd),
        Command::ToggleFriendly => toggle_friendly(hwnd),
        // The text pane's, and this one has no selection, block or outline to work with.
        Command::Bold
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
        | Command::InsertPicture => {}
        Command::Shortcuts => show_shortcuts(hwnd),
        Command::About => dialog::about(hwnd),
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
/// (`sheet::clip::rect_text`), and with `cut`, clear it afterwards — one `App::clear_range`, so
/// it is one undo step like Delete's.
///
/// What travels is each cell's `App::input_text` — the raw number, or a formula in display
/// form — rather than what the cell *displays*, for the reason `doc/windows-shell.md` decision
/// 6 gives: pasted back here it reproduces the cells exactly, and pasted into LibreOffice Calc
/// or Excel `1234.5` is a number where `1,234.50 €` is a guess about that program's locale.
fn copy(hwnd: HWND, cut: bool) {
    // SAFETY: one borrow; `clipboard::set_text` and `clear_range` each run with nothing else
    // borrowed.
    unsafe {
        with_sheet(hwnd, |state| {
            let (start, end) = state.selection.rect();
            let text = clip::rect_text(&state.app, state.sheet, start, end, App::input_text);
            clipboard::set_text(hwnd, &text);
            if cut && let Err(error) = state.app.clear_range(state.sheet, start, end) {
                state.say(Some(error.to_string()));
            }
        });
    }
    refresh(hwnd);
}

/// Read the clipboard and fill from the selection's top-left corner — `App::enter_range` under
/// `sheet::clip::parse_rows`, one undo step for the whole rectangle. Nothing happens when the
/// clipboard holds no text, which is what makes pasting an image or a file list silently do
/// nothing rather than write garbage into a cell.
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

// --- W6: the source, the check, and the two overlays ---

/// The document as its own projection (D9), opened already marked on the line the pane's own
/// selection or caret projects to — `Projection::line_of`, the reverse of the map `Show Source`'s
/// own list is built from. Read-only, and a modal list rather than a drawn pane: `code.rs`'s own
/// doc comment is why — this shell's dialog-for-a-list idiom is already `text_outline`'s and
/// `text_block_kind_dialog`'s, and a fourth text-view widget for one more read-only list would be
/// a second way of doing what `dialog::choose` already does.
fn show_source(hwnd: HWND) {
    // SAFETY: one borrow, released before the modal — which runs a nested message loop.
    let Some(projection) = (unsafe { with_pane(hwnd, project) }) else {
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

/// "Check Document" (D6) — every finding `App::lint` reports, worst first (`Report::diagnostics`
/// is already sorted), and every row a jump.
fn check_document(hwnd: HWND) {
    // SAFETY: one borrow, released before the modal.
    let Some(report) = (unsafe { with_pane(hwnd, lint) }) else {
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

/// Whether the formula bar shows the friendly *reading* of a formula rather than its text.
///
/// Menu-only, like the two overlays beside it, and the menu is rebuilt straight after for the
/// same reason: a checkmark answering with the state before the click is a checkmark that is
/// always one click behind.
fn toggle_friendly(hwnd: HWND) {
    // SAFETY: one borrow; nothing inside dispatches.
    unsafe {
        with_sheet(hwnd, |state| state.friendly = !state.friendly);
    }
    build_menu(hwnd);
    refresh(hwnd);
}

/// The function list: every function this build implements, and picking one writes the call.
///
/// `dialog::choose`'s listbox again — this shell's one idiom for "a list to pick a line from",
/// already the outline, the source and the lint pane. What the rows say is `assist::function_lines`,
/// which is `grind sheet functions --long`'s own four columns, so the window and the CLI cannot
/// disagree about what a function is called or what it does.
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

/// The active cell's formula, explained — `formula::friendly::explain`, one line per row.
///
/// Read-only: the answer is thrown away, exactly as Help ▸ Keyboard Shortcuts does with the same
/// dialog. It never parses back and nothing is written (R1) — this is a *reading* of the formula
/// the document stores, not a second spelling of it.
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
        // Not a formula, or one this build cannot parse. A sentence in the notice bar rather than
        // a modal saying no: the question was asked *about a cell*, and the bar is where this
        // window says things about the cell it is on.
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
    dialog::choose(hwnd, &format!("{address} explained"), &rows, 0);
}

/// `Pane::project`, spelled so [`with_pane`] can be handed it directly rather than a closure that
/// only repeats the match `Pane::sheet_mut`/`text_mut` already exist to avoid writing twice.
fn project(pane: &mut Pane) -> grind_core::projection::Projection {
    match pane {
        Pane::Sheet(sheet) => sheet.app.project(),
        Pane::Text(text) => text.app.project(),
    }
}

/// `Pane::lint`, over the default options — every rule, hints off, the same defaults `grind lint`
/// runs with no flags.
fn lint(pane: &mut Pane) -> grind_core::lint::Report {
    let options = grind_core::lint::Options::default();
    match pane {
        Pane::Sheet(sheet) => sheet.app.lint(&options),
        Pane::Text(text) => text.app.lint(&options),
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

/// Jump either pane to an address `Show Source` or `Check Document` produced.
///
/// The same two calls [`text_go_to`] already makes for the word processor, and `a1::parse` /
/// `a1::resolve` for the grid — **allowed to land on a different sheet than the one open**, unlike
/// `status::locate`'s name box: a diagnostic or a source line may be about any sheet in the
/// document, where a name box is answering "where on *this* sheet".
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

/// W7's "key list" — `menu::shortcuts()` in `dialog::choose`'s read-only listbox, the same
/// widget every other list in this shell already is. There is nowhere to jump to from an
/// accelerator, so the row picked (or Escape) is thrown away; the list exists to be read.
fn show_shortcuts(hwnd: HWND) {
    let rows = menu::shortcuts();
    let _ = dialog::choose(hwnd, "Keyboard Shortcuts", &rows, 0);
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

/// An autofilter over the selection, or clear the one the sheet already has — the Data menu's
/// *Autofilter*, `Grid::toggle_filter` in the GTK shell and `sheet.filter` in the web one
/// mirrored.
///
/// Over a sheet that already has one this clears it, so the command is the on/off switch its
/// name implies; otherwise the selection becomes the range, with its first row the heading.
/// Drawing the dropdown buttons and folding the rows it hides is `sheet/draw.rs`/`relayout`'s
/// (`win.rs:439-443` already merges `App::hidden_rows` into the hidden-row list every layout
/// pass); this is only the on/off switch and the range it covers.
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
            let (start, mut end) = state.selection.rect();
            // A single cell is a click, not a range — the same rule `format_table` uses.
            if start == end
                && let Ok((rows, cols)) = state.app.used_extent(state.sheet)
            {
                end = Pos::new(rows.saturating_sub(1), cols.saturating_sub(1));
            }
            if end.row <= start.row {
                return state.say(Some(
                    "Select the rows to filter, including their headings".to_owned(),
                ));
            }
            // The name LibreOffice gives an autofilter nobody named; `sheet filter` writes the
            // same one, so a document does not say which shell made it.
            let filter = Filter::new("__Anonymous_Sheet_DB__0", start, end);
            if let Err(error) = state.app.set_filter(state.sheet, Some(filter)) {
                state.say(Some(error.to_string()));
            }
        });
    }
    refresh(hwnd);
}

/// Open the dropdown for one field, under the button `button_down` caught —
/// `Click::FilterButton`'s handler.
///
/// Values come from a fresh read, the same range `ui_sheet_gtk`'s own `field_values` and the
/// web shell's build them from: the column's own cells from the filter's first data row to its
/// last, deduplicated. No borrow is held across `dialog::choose_multi`'s nested loop (decision
/// 7), so this is two separate `with_sheet` calls rather than one.
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

/// Format the selection as a table — the Data menu's *Format as Table*.
///
/// No dialog: the selection is the range, its first row is the heading, and the name
/// auto-generates (`Table1`, `Table2`, …), the same zero-prompt shape the web shell's
/// `sheet.format-table` and the GTK grid's `Grid::toggle_filter` both use for the plain case.
/// There is no "un-format" — ODF has nothing resembling a persisted table object to remove as
/// a unit — so clearing the effects means Undo (one step, right after this) or clearing the
/// filter, restyling the cells and dropping the name by hand later, same as after
/// LibreOffice's own AutoFormat (`sheet/src/table_format.rs`).
fn format_table(hwnd: HWND) {
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
                totals: false,
                name: None,
            };
            if let Err(error) = state.app.format_table(state.sheet, start, end, options) {
                state.say(Some(error.to_string()));
            }
        });
    }
    refresh(hwnd);
}

// --- sheets ---

/// Move to another sheet, keeping the selection and the view.
///
/// Clamped rather than wrapped: Ctrl+PageDown on the last sheet doing nothing is less surprising
/// than it jumping back to the first, and it is what Excel does.
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
    let Some(suggested) = (unsafe {
        with_sheet(hwnd, |state| {
            format!("Sheet{}", state.app.sheet_count() + 1)
        })
    }) else {
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
            // The answer is **how many references were rewritten**, not an index — D10's whole
            // point, and worth saying: a rename that quietly carried three hundred formulas
            // with it is a thing to be told about, and to be able to take back in one step.
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
    if !dialog::confirm(hwnd, &format!("Delete {name} and everything on it?")) {
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
    let (suggested, kind) = unsafe { with_pane(hwnd, |pane| (pane.path(), pane.kind())) }
        .unwrap_or((None, DocumentKind::Spreadsheet));
    let Some(path) = dialog::save_path(hwnd, suggested.as_deref(), kind) else {
        return false;
    };
    write(hwnd, &path)
}

/// Write the document. A failure is the one file operation that must never be quiet: the work is
/// still only in memory afterwards, and a close that went ahead anyway would lose it.
///
/// Which *form* is written is `Form::from_path`'s answer to the extension the user chose, inside
/// `grind_sheet::write_file` — the one place in the workspace where an extension decides
/// anything, and the reason this shell offers `.fods`, `.ods` and `.grind` and then says nothing
/// more about them.
fn write(hwnd: HWND, path: &Path) -> bool {
    // SAFETY: one borrow. `save_file` only reads the document: it notifies nothing, opens
    // nothing, and cannot dispatch a message.
    // No "Saved" banner: the title's `*` clearing is the confirmation, and routine success
    // asking to be noticed is noise.
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
            refresh(hwnd);
            true
        }
    }
}

/// Replace what the window is showing — **including with a document of the other kind**.
///
/// The observer is registered again because it is bound to an `App` rather than to the window: a
/// new document is a new `App`, and one nobody is listening to is one whose edits never mark the
/// title. Registering it *after* the file has been read is also why this shell needs no "a load
/// is not an edit" flag — during the read there is nobody to tell.
fn adopt(hwnd: HWND, pane: Pane) {
    // SAFETY: one borrow. `set_observer` stores an `Arc` and dispatches nothing, and dropping the
    // pane that was there releases its fonts and its document with no message in between.
    unsafe {
        with_pane(hwnd, |slot| {
            let observer = Arc::new(Changed(hwnd.0 as isize));
            match &pane {
                Pane::Sheet(sheet) => sheet.app.set_observer(observer),
                Pane::Text(text) => text.app.set_observer(observer),
            }
            *slot = pane;
        });
    }
    // A window that has just become a spreadsheet needs the two `EDIT`s a spreadsheet edits
    // through; one that has just become a document needs nothing and gets nothing.
    make_children(hwnd);
    // The menu is greyed by pane kind (`build_menu`), and File ▸ Open or File ▸ New can change
    // that kind in the same window — so it is rebuilt here rather than only once at `WM_CREATE`.
    build_menu(hwnd);
    refresh(hwnd);
}

/// A new, empty document **of the kind the window is already showing**.
///
/// Not a choice, and that is deliberate for now: a New Spreadsheet / New Document pair is a menu
/// question, and W7 is where the menus are finished. `grind-win32 --text` and File ▸ Open reach
/// the other kind today, which is the R9 answer in the meantime.
fn new_document(hwnd: HWND) {
    if !offer_to_save(hwnd) {
        return;
    }
    // SAFETY: one borrow, released before anything else happens.
    let kind = unsafe { with_pane(hwnd, |pane| pane.kind()) };
    let Some(kind) = kind else { return };
    // SAFETY: one borrow, for the theme the new pane starts in.
    let theme = unsafe { with_pane(hwnd, |pane| pane.theme()) }.unwrap_or_else(theme::current);
    if let Ok(pane) = opened(kind, None, theme) {
        adopt(hwnd, pane);
    }
}

fn open_document(hwnd: HWND) {
    if !offer_to_save(hwnd) {
        return;
    }
    let Some(path) = dialog::open_path(hwnd) else {
        return;
    };
    // The kind is read from the bytes rather than from the name, before anything is parsed —
    // `grind_core::kind`, and `main.rs` asks it the same question about the command line. **This
    // is where one binary holding both document types stops being a claim**: a `.fodt` chosen
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
    match opened(kind, Some(path.clone()), theme) {
        Ok(pane) => adopt(hwnd, pane),
        Err(error) => dialog::error(
            hwnd,
            &format!("Could not open {}:\n\n{error}", path.display()),
        ),
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

/// Everything one frame needs, drawn onto a device context.
///
/// Takes an `HDC` and the state and nothing about the window, which is what makes
/// [`render`] a second *caller* rather than a second drawing path.
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
    let (status, status_right) = status::status_halves(&state.app, state.sheet, state.selection);
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
    draw::paint(
        dc,
        &Frame {
            geom: &state.geom,
            theme: state.theme,
            viewport: &viewport,
            status: &status,
            status_right: &status_right,
            name: &name,
            formula: friendly.as_deref().unwrap_or(&formula),
            friendly: friendly.is_some(),
            banner: state.banner.as_deref(),
            hint: &state.hint,
            selection: state.selection,
            filter: state.app.filter(state.sheet).ok().flatten(),
            used: state.app.used_extent(state.sheet).unwrap_or((0, 0)),
            font_px: scale(theme::text::CELL, state.geom.dpi).round() as i32,
            caption_px: scale(theme::text::CAPTION, state.geom.dpi).round() as i32,
            body_px: scale(theme::text::BODY, state.geom.dpi).round() as i32,
            face: face(),
        },
    );
}

/// One frame, through the back buffer.
fn paint(hwnd: HWND) {
    // The text pane's caret is Windows' own object now (`place_system_caret`), and the blit
    // below is unaware of it — `BitBlt` paints over whatever was on screen, caret included,
    // which is what turns it into a stray rectangle if it is not hidden first and shown again
    // after. The grid never creates one, so this is a no-op there.
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
                }
            });
            buffer.present(dc);
        }
        let _ = EndPaint(hwnd, &ps);
    }
    if has_caret {
        // Repositioned here rather than trusted to whatever handler invalidated the window: a
        // caret this pane moves in a dozen places is a caret placed in one, the same argument
        // `App::caret_line` already won for *where* it goes — this is only *that* it is shown
        // there again after every single repaint, including ones with nothing to do with the
        // caret at all (a resize, a theme change), which costs nothing next to a whole frame.
        place_system_caret(hwnd);
    }
}

// --- the text pane (W5) ---
//
// Everything below belongs to the word processor's pane. The shape is deliberately the same as
// the grid's above — a refresh that rebuilds everything derived, a frame that takes an `HDC` and
// no `HWND`, handlers that borrow the state for the length of one message — so that the two panes
// are two documents in one window rather than two programs in one binary.

/// Whether this window is showing a document rather than a spreadsheet.
///
/// Asked at the top of each shared handler rather than by giving every message two arms, because
/// most of them *are* one pane's: the grid's forty handlers kept the shape they had.
fn is_text(hwnd: HWND) -> bool {
    // SAFETY: one borrow, for one bit; nothing inside dispatches.
    unsafe { with_pane(hwnd, |pane| matches!(pane, Pane::Text(_))) }.unwrap_or(false)
}

/// Show the system caret at [`Text::caret_geometry`], creating it first.
///
/// `CreateCaret` again rather than only `SetCaretPos`, because the caret is a different height on
/// a heading than on a paragraph and recreating is how either is set — simpler than tracking
/// whether the size actually changed, and Windows already blinks it at the user's own
/// `GetCaretBlinkTime` for free, which is what makes a system caret worth having at all: this
/// shell no longer keeps that setting or a timer of its own. The ordinary cost of recreating on
/// every keystroke is that Windows resets the blink phase each time, which reads as normal
/// because every editor's own caret does the same thing.
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
    // The one place this is called from every motion and every edit — `place_system_caret`
    // recreates the caret at the caret's current size and position, which is cheap next to a
    // whole-window invalidate and correct even though a keystroke that does not move the caret
    // calls it too: `CreateCaret` with no change is documented as harmless.
    place_system_caret(hwnd);
}

/// The scrollbars for a document, which are **pixels** where the grid's are tracks.
///
/// That difference is the reason this is not `sync_scrollbars` with a flag: a sheet is twenty
/// million pixels tall and only a million rows, so it has to scroll in tracks; a document is as
/// tall as it measures, and a line is not a unit anything else in this pane counts in. The
/// horizontal bar is disabled outright — the measure is fixed, so there is nothing to scroll to.
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

/// A character, after the keyboard layout, the IME and — care of `typed_char` — any surrogate
/// pair it arrived as.
///
/// Control characters arrive here too — Escape is `\u{1b}` and Enter is `\r`, because
/// `TranslateMessage` produces a `WM_CHAR` for both — and none of them is text. Return and
/// Backspace are handled as *keys* above, which is where they belong.
fn text_char(hwnd: HWND, c: char) -> bool {
    let m = mods();
    if m.ctrl || m.alt || c.is_control() {
        return false;
    }
    text_type(hwnd, c);
    true
}

/// Type one character through `App::type_markdown`, so `**bold**` is read as it is typed —
/// `grind_text::markdown`'s notation, the same one `ui_tui` reads, rather than a fifth idea of
/// what `**` means.
///
/// A selection is dropped first exactly as [`text_insert`] drops one, because `type_markdown`
/// has no "replace" either — it only ever inserts at a caret.
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
                    text.resume = typed.resume;
                    text.place(typed.caret, false);
                    text.caret_on = true;
                    text.say(None);
                }
                Err(error) => text.say(Some(error.to_string())),
            }
        });
    }
    refresh(hwnd);
}

/// Type text at the caret, replacing the selection if there is one.
///
/// One `Action::Batch` per keystroke would be one Ctrl+Z per keystroke; that the erase and the
/// insert are two is a named gap, not an oversight — `App` has no "replace" and adding one is a
/// core change rather than a shell one.
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

/// Erase whatever is selected, leaving the caret where it was. `true` when there was something.
///
/// A free function rather than a method because it is the *editing* half of the selection and
/// every edit begins with it: typing over a selection replaces it, which is what every editor
/// does and the one thing a shell must not forget.
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

/// A press of the left button: the format strip's three buttons first, since they sit over the
/// document rather than beside it — a click there is a click on chrome and must not also move
/// the caret underneath it, the same rule `button_down`'s own `Strip` match applies to the grid's
/// name box and formula bar. Anywhere else, the caret goes where the pointer is, and dragging
/// extends.
fn text_button_down(hwnd: HWND, lparam: LPARAM) {
    let (x, y) = point(lparam);
    // SAFETY: one borrow; the hit test is arithmetic over the page geometry and dispatches
    // nothing.
    let hit = unsafe { with_text(hwnd, |text| text.page.strip_hit(x, y)) }.flatten();
    if let Some(hit) = hit {
        // **Pressed here, done on release** (W10) — which is what every button on this platform
        // does and is not only cosmetic: a press that has not been let go of can be taken back by
        // dragging off the control, and until W10 this strip acted the instant the button went
        // down, so a mis-aimed click was a formatting edit and a Ctrl+Z.
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
            // The strip's hover (W10), asked on every move and repainted only when the answer
            // *changes* — which is what keeps a move across the document down to one comparison.
            // `WM_MOUSELEAVE` is not tracked and does not need to be: the pointer cannot leave
            // the window from inside the strip without passing over the rest of it first, and
            // any point that is not on a control clears the hover anyway.
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

/// A verb, in the text pane. Exhaustive on [`Command`] for the same reason the grid's is: a verb
/// added to `menu.rs` and nowhere else fails the build rather than doing nothing.
///
/// The verbs that are the *spreadsheet's* are no-ops here and say nothing about it — Recalculate
/// and the four sheet verbs cannot be greyed out until W7 makes the menus state-aware, and a
/// message box saying "this document has no sheets" would be worse than nothing happening.
fn text_command(hwnd: HWND, command: Command) {
    match command {
        Command::New => new_document(hwnd),
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
        Command::Outline => text_outline(hwnd),
        Command::BlockKindDialog => text_block_kind_dialog(hwnd),
        Command::InsertPicture => text_insert_picture(hwnd),
        Command::ShowSource => show_source(hwnd),
        Command::CheckDocument => check_document(hwnd),
        Command::ToggleNames => text_toggle_names(hwnd),
        Command::Shortcuts => show_shortcuts(hwnd),
        Command::About => dialog::about(hwnd),
        // The spreadsheet's, and this pane has no answer to any of them: it has no sheets, no
        // cells for `CellRole` to classify, and no formulas to list, explain or read out.
        Command::Recalculate
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

/// Go to an address — `p12`, `#intro` or `§2.1.3` — the one item of W5b's "block kinds, outline
/// and go-to" bullet this pane has so far. A modal prompt rather than a strip box, unlike the
/// grid's name box, because this pane owns no child control of its own to put one in; `loc::parse`
/// and `App::resolve_caret` are the same two calls `ui_tui`'s `cmd_jump` makes.
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

/// The outline dialog: every heading, indented by its own depth, jump to any of them.
///
/// `App::outline` is the list and `Heading::address` the same `§2.1.3` spelling `text_go_to`'s
/// prompt takes, but this goes straight to the block index it already has rather than round
/// tripping through `loc::parse` for an address it just built.
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

/// The property this emphasis lives in, read off a style rather than written to one — the half of
/// [`text_emphasise`]'s old inline closure that answering "is this on" and "what should it become"
/// both need, and now shared between the two rather than duplicated for the strip.
fn emphasis_field(style: &grind_text::CharStyle, emphasis: markdown::Emphasis) -> Option<String> {
    match emphasis {
        markdown::Emphasis::Bold => style.font_weight.clone(),
        markdown::Emphasis::Italic => style.font_style.clone(),
        markdown::Emphasis::Underline => style.underline.clone(),
        markdown::Emphasis::Strike => style.line_through.clone(),
        markdown::Emphasis::Code => style.font_family.clone(),
    }
}

/// The value that means "off", for the three that have one — `Code` sets a family, which has no
/// off value of its own, only "no family" (`None`).
fn emphasis_off(emphasis: markdown::Emphasis) -> Option<&'static str> {
    match emphasis {
        markdown::Emphasis::Bold | markdown::Emphasis::Italic => Some("normal"),
        markdown::Emphasis::Underline | markdown::Emphasis::Strike => Some("none"),
        markdown::Emphasis::Code => None,
    }
}

/// Whether `style` already has this emphasis — the question a toggle button answers before it
/// decides which way to toggle, and the same one its own drawing asks to decide whether to press
/// itself in.
fn emphasis_active(style: &grind_text::CharStyle, emphasis: markdown::Emphasis) -> bool {
    let field = emphasis_field(style, emphasis);
    match emphasis_off(emphasis) {
        Some(off) => field.as_deref().is_some_and(|v| v != off),
        None => field.is_some(),
    }
}

/// The style a toggle button reads before it draws itself: the selection's agreed style if there
/// is one, or — with nothing selected — the style the *next* character typed would carry, which is
/// [`Text::resume`] when a markdown span left one pending and otherwise the character just behind
/// the caret. A document has no style "at" an empty caret; it has one on either side of it, and
/// the one already typed is the one a toolbar showing current state means.
fn text_style_here(text: &Text) -> grind_text::CharStyle {
    if text.has_selection() {
        let (from, to) = text.range();
        return text.app.char_style(from, to).unwrap_or_default();
    }
    if let Some(resume) = &text.resume {
        return resume.clone();
    }
    let caret = text.caret;
    if caret.offset == 0 {
        return grind_text::CharStyle::default();
    }
    let before = Caret {
        block: caret.block,
        offset: caret.offset - 1,
    };
    text.app.char_style(before, caret).unwrap_or_default()
}

/// Which of the strip's five toggles should be drawn pressed in, for [`text::draw::paint`].
fn format_state(text: &Text) -> [bool; 5] {
    let style = text_style_here(text);
    STRIP_BUTTONS.map(|emphasis| emphasis_active(&style, emphasis))
}

/// Toggle one emphasis across the selection — the format strip `doc/windows-shell.md` named as
/// still owed for W5b, and now drawn (`text::draw::paint`'s strip) as well as reachable from
/// Ctrl+B/I/U and the Format menu, all three converging on this one function. `App::char_style`
/// reports only what the whole span agrees on, which is the same question a toggle button would
/// ask; `ui_tui::emphasise_selection` is the twin this mirrors so the notation reads one way
/// everywhere.
fn text_emphasise(hwnd: HWND, emphasis: markdown::Emphasis) {
    // SAFETY: one borrow. `set_char_style` notifies, and the observer posts rather than sends.
    unsafe {
        with_text(hwnd, |text| {
            if !text.has_selection() {
                text.say(Some("nothing selected".to_owned()));
                return;
            }
            let (from, to) = text.range();
            let mut style = text.app.char_style(from, to).unwrap_or_default();
            let wanted = emphasis.style();
            let field = |style: &grind_text::CharStyle| emphasis_field(style, emphasis);
            let off = emphasis_off(emphasis);
            let already = emphasis_active(&style, emphasis);
            let value = match already {
                true => off.map(str::to_owned),
                false => field(&wanted),
            };
            match emphasis {
                markdown::Emphasis::Bold => style.font_weight = value,
                markdown::Emphasis::Italic => style.font_style = value,
                markdown::Emphasis::Underline => style.underline = value,
                markdown::Emphasis::Strike => style.line_through = value,
                markdown::Emphasis::Code => style.font_family = value,
            }
            match text.app.set_char_style(from, to, &style) {
                Ok(_) => text.say(None),
                Err(error) => text.say(Some(error.to_string())),
            }
        });
    }
    refresh(hwnd);
}

/// Apply a formatting-bar change over the selection — [`text_emphasise`]'s general form, over
/// [`grind_text::format::Change`] rather than one `markdown::Emphasis`, for the strip's four
/// controls that write more than a boolean: Family, Size, the two swatches and Clear. The same
/// vocabulary `grind-text-gtk`'s `format.rs` writes through, hoisted into `grind-text` itself so
/// the two shells cannot disagree about what a control does.
fn text_format(hwnd: HWND, change: grind_text::format::Change) {
    // SAFETY: one borrow. `set_char_style` notifies, and the observer posts rather than sends.
    unsafe {
        with_text(hwnd, |text| {
            if !text.has_selection() {
                text.say(Some("nothing selected".to_owned()));
                return;
            }
            let (from, to) = text.range();
            let mut style = text.app.char_style(from, to).unwrap_or_default();
            change.apply(&mut style);
            match text.app.set_char_style(from, to, &style) {
                Ok(_) => text.say(None),
                Err(error) => text.say(Some(error.to_string())),
            }
        });
    }
    refresh(hwnd);
}

/// The families the picker offers — curated rather than enumerated. `grind-text-gtk` lists every
/// family Pango can resolve; this shell has no `EnumFontFamiliesExW` wiring yet, which is a named
/// gap in `doc/windows-shell.md` rather than an oversight. The document's own family is added
/// when it set one this list does not carry, so a document written elsewhere still shows what it
/// chose.
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
    let items: Vec<String> = std::iter::once("Automatic".to_owned())
        .chain(
            grind_core::style::PALETTE
                .iter()
                .map(|(name, _)| (*name).to_owned()),
        )
        .collect();
    let initial = current
        .as_deref()
        .and_then(|hex| {
            grind_core::style::PALETTE
                .iter()
                .position(|(_, value)| *value == hex)
        })
        .map(|index| index + 1)
        .unwrap_or(0);
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

/// Turn the caret's own block into this kind, wearing this named paragraph style —
/// `App::set_kind` and `App::set_style` in one gesture, `ui_text_gtk`'s own `Ui::set_kind`
/// mirrored here so a heading and a *Title* behave one way everywhere rather than a
/// shell-specific idea of either.
///
/// **The style is only ever taken off when this window put it on** — `grind_text::named_style_for`
/// is the whole rule, shared with that window: choosing Paragraph after *Title* or *Subtitle*
/// clears the name, and a document's own name this build does not interpret (`Quotations`, say)
/// is left exactly as it is.
///
/// Unlike [`text_emphasise`], which needs a selection, a block's kind is asked of wherever the
/// caret sits — the same reason `App::set_kind` takes an index and not a range.
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

/// Tab or Shift+Tab: nest a list item one level deeper, un-nest one, or start one at the front
/// of a block — `grind_text::indent_kind` is the whole rule, the same one `ui_text_gtk`'s
/// `Doc::indent` calls, so a list nests the same way in both windows. A literal tab character
/// where none of that applies, which is what a word processor's Tab does in the middle of a
/// sentence; Shift+Tab with nothing to un-nest types nothing, since there is no such character.
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

/// Every block kind, listed rather than one key per depth — the schema's headings go past level
/// 3 and lists have a depth of their own, and `Command::Heading1`/`2`/`3` only reach the common
/// case. `dialog::choose` is the same `LISTBOX` popup `text_outline` already opens, generic over
/// strings for the same reason: it has no idea what a `BlockKind` is and does not need one.
///
/// This is the first window in the suite to reach a list item from its own UI at all —
/// `doc/text-shell.md` names "no lists UI" as a gap in both GTK windows and the browser, since a
/// list read from a file draws its bullet but nothing makes one. Six heading levels plus four
/// list depths plus Paragraph is the same authoring ceiling `doc/text-core.md` already draws
/// (headings) or is a reasonable one to draw (`MAX_LIST_DEPTH`) rather than the schema's
/// literally uncapped nesting.
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

/// Insert a picture at the caret — a file dialog, then `App::insert_image`, mirroring
/// `ui_text_gtk`'s own `Ui::embed_image`: an empty block is used as it stands, and any other
/// block gets a fresh paragraph inserted after it, so the picture always lands in a paragraph of
/// its own rather than mid-sentence, where every shell in the suite still draws the placeholder
/// character instead of the picture.
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
            let block = text.caret.block;
            let empty = text
                .app
                .input_text(block)
                .is_ok_and(|content| content.is_empty());
            let at = match empty {
                true => block,
                false => {
                    if let Err(error) =
                        text.app
                            .insert(block + 1, grind_text::BlockKind::Paragraph, "")
                    {
                        return Err(error.to_string());
                    }
                    block + 1
                }
            };
            text.app
                .insert_image(
                    Caret {
                        block: at,
                        offset: 0,
                    },
                    mime,
                    data,
                    None,
                    None,
                )
                .map(|()| at)
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

/// Copy the selection to the clipboard as `CF_UNICODETEXT`, and with `cut`, erase it afterwards.
///
/// What travels is the document's **plain text**, one `\r\n` per block: the shape every other
/// program on this platform reads. The formatting does not travel, which is the honest position
/// for a build with no `CF_RTF` or `HTML Format` writer — and a named gap rather than a silent
/// one, because pasting into WordPad and losing the bold is the sort of thing a user attributes
/// to the document.
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

/// Everything one frame of the text pane needs, drawn onto a device context.
///
/// Takes an `HDC` and the state and nothing about the window, which is what makes [`render`] a
/// second *caller* rather than a second drawing path — the same property the grid's `draw_frame`
/// has, and the reason `--render-to` works for a document too.
/// `system_caret` is `false` only for [`render`]'s windowless frame, which has no `HWND` and so
/// no system caret to draw one for it — every other caller has a real window and Windows draws
/// its own caret over whatever this paints, so drawing one here too would be two.
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
    let status = text::status::status_line(
        &grind_text::loc::format_offset(state.caret.block, state.caret.offset),
        selected_chars(state),
        state.app.counts(),
    );
    let style = text_style_here(state);
    text::draw::paint(
        dc,
        &text::draw::Frame {
            page: &state.page,
            theme: state.theme,
            faces: &faces,
            blocks: &blocks,
            height: state.flow.height(),
            selection: state.range(),
            caret: state.caret,
            caret_on: state.caret_on && !system_caret,
            status: &status,
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
        },
    );
}

/// How many characters the selection covers — what the status bar says.
///
/// Counted rather than subtracted, because a selection that spans blocks covers the breaks
/// between them as well, and each of those is one character a `Caret` can sit either side of.
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
