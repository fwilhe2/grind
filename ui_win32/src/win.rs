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
//!    [`ask`] is that shape as a function, so the pattern is named rather than remembered.
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

use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, EndPaint, HDC, InvalidateRect, OPAQUE, PAINTSTRUCT, SetBkColor, SetBkMode,
    SetTextColor, UpdateWindow,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForWindow, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CREATESTRUCTW, CS_DBLCLKS, CW_USEDEFAULT, CreateMenu, CreatePopupMenu,
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, EN_CHANGE, EN_KILLFOCUS,
    ES_AUTOHSCROLL, GWLP_USERDATA, GetMessageW, GetParent, GetWindowLongPtrW, GetWindowTextLengthW,
    GetWindowTextW, HMENU, IDC_ARROW, LoadCursorW, MF_POPUP, MF_SEPARATOR, MF_STRING, MSG,
    MoveWindow, PostMessageW, PostQuitMessage, RegisterClassW, SB_BOTTOM, SB_HORZ, SB_LINEDOWN,
    SB_LINEUP, SB_PAGEDOWN, SB_PAGEUP, SB_THUMBPOSITION, SB_THUMBTRACK, SB_TOP, SB_VERT,
    SCROLLINFO, SCROLLINFO_MASK, SIF_PAGE, SIF_POS, SIF_RANGE, SPI_GETWHEELSCROLLLINES, SW_HIDE,
    SW_SHOW, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOZORDER, SendMessageW, SetMenu, SetTimer,
    SetWindowLongPtrW, SetWindowPos, SetWindowTextW, ShowWindow, SystemParametersInfoW,
    TranslateMessage, WHEEL_DELTA, WM_APP, WM_CHAR, WM_CLOSE, WM_COMMAND, WM_CREATE,
    WM_CTLCOLOREDIT, WM_DESTROY, WM_DPICHANGED, WM_ERASEBKGND, WM_HSCROLL, WM_KEYDOWN,
    WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_NCCREATE,
    WM_NCDESTROY, WM_PAINT, WM_SETFONT, WM_SETTINGCHANGE, WM_SIZE, WM_TIMER, WM_VSCROLL, WNDCLASSW,
    WS_CHILD, WS_HSCROLL, WS_OVERLAPPEDWINDOW, WS_VSCROLL,
};
// Focus and mouse capture are Windows' input API rather than its window-management one, which
// is where its own metadata puts them.
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, ReleaseCapture, SetCapture, SetFocus, VK_CONTROL, VK_MENU, VK_SHIFT,
};
// The caret's blink rate is the *user's* setting and lives with the rest of the caret API.
use windows::Win32::UI::WindowsAndMessaging::GetCaretBlinkTime;
// `SetScrollInfo` lives in the Controls namespace in Windows' own metadata, which is where the
// scrollbar API has always been. It is *not* a Common Controls v6 class and needs no manifest —
// `doc/windows-shell.md`'s rejection of a v6 toolbar does not reach it.
use windows::Win32::UI::Controls::{EM_SETSEL, SetScrollInfo};
use windows::core::PCWSTR;

use crate::clipboard;
use crate::dialog::{self, Answer, Com};
use crate::gdi::{self, BackBuffer, Brush, Dib, Font};
use crate::menu::{self, Command, Item};
use crate::metrics::{Faces, Fonts};
use crate::notice;
use crate::sheet::clip;
use crate::sheet::draw::{self, Frame};
use crate::sheet::geom::{GridGeom, Hit, MAX_COLS, MAX_ROWS, Rect, Sizes, scale};
use crate::sheet::keymap::{self, Dir, Selection};
use crate::sheet::state::{self, Outcome, Seed};
use crate::sheet::status;
use crate::text;
use crate::text::geom::{Flow, Page};
use crate::theme::{self, Mode, Theme};
use grind_core::DocumentKind;
use grind_sheet::{App, Pos, RecalcMode};
use grind_text::{Caret, Layout};

/// The display name, which is not the file name (`doc/windows-shell.md`, decision 1).
const APP_NAME: &str = "Grind";
const CLASS_NAME: &str = "GrindWindowClass";

/// The face the grid is drawn in.
///
/// Segoe UI is the shell font on every Windows this shell targets. GDI substitutes when it is
/// absent — which is what happens under Wine, and is the substitution path
/// `doc/windows-shell.md` says every screenshot taken there exercises.
const FACE: &str = "Segoe UI";
/// The grid's text size at 100%, in pixels.
const FONT_PX: f64 = 13.0;

/// The two child `EDIT`s, one id each — `WM_COMMAND`'s low word is how a notification says
/// which child it came from. Both are below [`menu::FIRST_ID`], which is what keeps a control's
/// notification and a menu item's click apart in the one message Win32 uses for both.
const ID_NAME_BOX: usize = 1;
const ID_EDITOR: usize = 2;

/// The size prose is set at, in pixels at 100%.
///
/// Larger than the grid's [`FONT_PX`] on purpose: a spreadsheet is read by scanning a table and a
/// document is read by reading it, and every word processor there has ever been sets body text
/// bigger than a cell. Headings scale off this (`metrics.rs`).
const TEXT_PX: i32 = 15;

/// The caret's blink, as a timer id. One per window, and the interval is `GetCaretBlinkTime` —
/// the user's own setting rather than a constant, which is the difference between drawing a caret
/// and inventing one.
const ID_CARET_TIMER: usize = 1;

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
/// [`grind_core::kind`] reading the file's bytes, and every message below either belongs to one
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
    /// Always set through [`State::say`], which is what keeps it and `geom.banner_h` agreeing.
    banner: Option<String>,
    /// The face the two child `EDIT`s are set in. Owned here because a `WM_SETFONT` does not
    /// take a copy: the handle has to outlive every paint of the control, and be deleted after
    /// it.
    ui_font: Option<Font>,
    /// The ground the child `EDIT`s are painted on, answered to `WM_CTLCOLOREDIT`. Built on
    /// demand and thrown away when the theme changes, because a brush is a colour and the
    /// colour is the theme's.
    field_brush: Option<Brush>,
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
    /// The blink's phase. The *interval* is `GetCaretBlinkTime` — the user's own setting, which
    /// a shell that drew its own caret would otherwise quietly override.
    caret_on: bool,
    banner: Option<String>,
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
        self.flow = text::geom::flow_of(&self.app, &faces, self.page.dpi);
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
        ui_font: None,
        field_brush: None,
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
) -> Result<(), String> {
    let mut pane = opened(kind, path, Theme::of(Mode::Light))?;
    let (w, h) = (f64::from(RENDER_W), f64::from(RENDER_H));
    let dib = Dib::new(RENDER_W, RENDER_H).ok_or("could not make the drawing surface")?;
    match &mut pane {
        Pane::Sheet(sheet) => {
            sheet.relayout(w, h, 96);
            draw_frame(dib.dc(), sheet);
        }
        Pane::Text(text) => {
            text.relayout(w, h, 96);
            draw_text_frame(dib.dc(), text);
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
                    theme::apply_title_bar(hwnd, theme);
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
            start_blinking(hwnd);
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
                    SetBkColor(dc, COLORREF(state.theme.field.colorref()));
                    let field = state.theme.field;
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
        // The caret's blink, and nothing else uses a timer.
        WM_TIMER if wparam.0 == ID_CARET_TIMER => {
            blink(hwnd);
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
                    theme::apply_title_bar(hwnd, theme);
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
fn build_menu(hwnd: HWND) {
    // SAFETY: every label buffer outlives the `AppendMenuW` that reads it — Windows copies the
    // string — and the bar belongs to the window from `SetMenu` until it is destroyed with it.
    unsafe {
        let Ok(bar) = CreateMenu() else { return };
        for menu in menu::MENUS {
            let Ok(popup) = CreatePopupMenu() else {
                continue;
            };
            for item in menu.items {
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
                    }
                }
            }
            let title = gdi::wide(menu.title);
            let _ = AppendMenuW(bar, MF_POPUP, popup.0 as usize, PCWSTR(title.as_ptr()));
        }
        let _ = SetMenu(hwnd, Some(bar));
    }
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
        let font = Font::new(FACE, scale(FONT_PX, dpi).round() as i32, false);
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
        Some(Focused::Editor(mode)) => on_key(hwnd, mode, vk),
        None => false,
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

/// A character, after the keyboard layout and after the IME. `true` means it started an edit.
///
/// ponytail: a character outside the BMP arrives as two `WM_CHAR`s carrying one surrogate each,
/// and `char::from_u32` refuses both — so an emoji cannot *start* an edit, though it types
/// perfectly well into one that is already open, because the control assembles the pair itself.
/// The fix is to hold a pending high surrogate here; it is not written because W5's `WM_IME_*`
/// path is where that state belongs, and doing it now would be a second copy of it.
fn typed_char(hwnd: HWND, code: u32) -> bool {
    let Some(c) = char::from_u32(code) else {
        return false;
    };
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
    let strip = unsafe {
        with_sheet(hwnd, |state| {
            let hit = state.geom.hit(x, y);
            if hit == Hit::Chrome {
                return if state.geom.name_box_rect().contains(x, y) {
                    Strip::NameBox
                } else if state.geom.formula_rect().contains(x, y) {
                    Strip::FormulaBar
                } else {
                    Strip::Neither
                };
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
            Strip::Neither
        })
    };
    match strip {
        Some(Strip::NameBox) => {
            open_name_box(hwnd);
            return;
        }
        // Clicking the bar edits the cell it is showing, which is the one place an edit begins
        // somewhere other than on the cell itself.
        Some(Strip::FormulaBar) => {
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
        // SAFETY: one borrow, and nothing inside it dispatches.
        unsafe {
            with_text(hwnd, |text| text.dragging = false);
            let _ = ReleaseCapture();
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
/// `on_bar` forces it onto the formula bar; otherwise [`State::editor_rect`] decides, and picks
/// the bar anyway for a cell that is scrolled out of sight.
fn begin_edit(hwnd: HWND, seed: Seed, on_bar: bool) {
    // SAFETY: one borrow, for the control, its rectangle and its text.
    let Some((edit, rect, text)) = (unsafe {
        with_sheet(hwnd, |state| {
            state.mode = seed.mode();
            state.editor_on_bar = on_bar;
            state.drag = None;
            let text = match seed {
                Seed::Char(c) => c.to_string(),
                Seed::Cell => status::formula_bar_text(&state.app, state.sheet, state.selection),
            };
            (state.editor, state.editor_rect(), text)
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
            // Whatever the banner was saying, it was about this edit.
            state.say(None);
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
/// is repainted. **Only the strip**: a keystroke must not redraw the grid under it.
fn editor_changed(hwnd: HWND) {
    // SAFETY: one borrow, released before the invalidation.
    let Some(strip) = (unsafe { with_sheet(hwnd, |state| state.geom.strip_rect()) }) else {
        return;
    };
    let (left, top, right, bottom) = strip.edges();
    let rect = RECT {
        left,
        top,
        right,
        bottom,
    };
    // SAFETY: the rectangle is a live local read for the length of the call.
    unsafe {
        let _ = InvalidateRect(Some(hwnd), Some(&rect), false);
    }
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
        Command::SheetAdd => sheet_add(hwnd),
        Command::SheetRename => sheet_rename(hwnd),
        Command::SheetDelete => sheet_delete(hwnd),
        Command::SheetNext => sheet_step(hwnd, 1),
        Command::SheetPrevious => sheet_step(hwnd, -1),
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
    let suggested = unsafe { with_pane(hwnd, |pane| pane.path()) }.flatten();
    let Some(path) = dialog::save_path(hwnd, suggested.as_deref()) else {
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
    let kind = unsafe {
        with_pane(hwnd, |pane| match pane {
            Pane::Sheet(_) => DocumentKind::Spreadsheet,
            Pane::Text(_) => DocumentKind::Text,
        })
    };
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
        .get_viewport(state.sheet, rows, cols)
        .unwrap_or_else(|_| {
            state
                .app
                .get_viewport(0, 0..0, 0..0)
                .expect("an empty rectangle of the first sheet always reads")
        });
    let status = status::status_line(&state.app, state.sheet, state.selection);
    let name = status::name_box_text(&state.app, state.sheet, state.selection);
    // While an in-cell edit is open the bar mirrors the control, which is what makes the strip a
    // read-out of *the cell* rather than of the document underneath it. When the control is on
    // the bar it is covering this text, and reading it back would be drawing under a window.
    let formula = match state.mode.is_editing() && !state.editor_on_bar {
        true => window_text(state.editor),
        false => status::formula_bar_text(&state.app, state.sheet, state.selection),
    };
    draw::paint(
        dc,
        &Frame {
            geom: &state.geom,
            theme: state.theme,
            viewport: &viewport,
            status: &status,
            name: &name,
            formula: &formula,
            banner: state.banner.as_deref(),
            selection: state.selection,
            font_px: scale(FONT_PX, state.geom.dpi).round() as i32,
            face: FACE,
        },
    );
}

/// One frame, through the back buffer.
fn paint(hwnd: HWND) {
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
                    Pane::Text(text) => draw_text_frame(buffer.dc(), text),
                }
            });
            buffer.present(dc);
        }
        let _ = EndPaint(hwnd, &ps);
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

/// Start the caret blinking at the user's own rate.
///
/// `GetCaretBlinkTime` is the setting the keyboard control panel writes, and honouring it is the
/// difference between drawing a caret and inventing one. `INFINITE` means the user turned
/// blinking off — an accessibility setting, and one this shell obeys by never starting a timer.
fn start_blinking(hwnd: HWND) {
    if !is_text(hwnd) {
        return;
    }
    // SAFETY: no arguments; the timer is destroyed with the window.
    unsafe {
        let interval = GetCaretBlinkTime();
        if interval == 0 || interval == u32::MAX {
            return;
        }
        SetTimer(Some(hwnd), ID_CARET_TIMER, interval, None);
    }
}

/// One blink. Only the caret's own line is worth repainting, but the window is small and the
/// frame is double-buffered, so the whole of it is invalidated — measured as imperceptible under
/// Wine, and the alternative is a rectangle that has to be kept in step with the layout.
fn blink(hwnd: HWND) {
    // SAFETY: one borrow, for one bit; nothing inside dispatches.
    unsafe {
        with_text(hwnd, |text| text.caret_on = !text.caret_on);
        let _ = InvalidateRect(Some(hwnd), None, false);
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
        text::keymap::Action::Tab => text_insert(hwnd, "\t"),
    }
    true
}

/// A character, after the keyboard layout and after the IME.
///
/// Control characters arrive here too — Escape is `\u{1b}` and Enter is `\r`, because
/// `TranslateMessage` produces a `WM_CHAR` for both — and none of them is text. Return and
/// Backspace are handled as *keys* above, which is where they belong.
///
/// ponytail: a character outside the basic multilingual plane arrives as two `WM_CHAR`s carrying
/// one surrogate each and `char::from_u32` refuses both, so an emoji cannot be typed here. The fix
/// is to hold a pending high surrogate in the pane; it belongs with the `WM_IME_*` path, which is
/// W5's remaining half, and doing it now would be a second copy of that state.
fn text_char(hwnd: HWND, c: char) -> bool {
    let m = mods();
    if m.ctrl || m.alt || c.is_control() {
        return false;
    }
    text_insert(hwnd, &c.to_string());
    true
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

/// A press of the left button: the caret goes where the pointer is, and dragging extends.
fn text_button_down(hwnd: HWND, lparam: LPARAM) {
    let (x, y) = point(lparam);
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
            if !text.dragging {
                return false;
            }
            match text.caret_at(x, y) {
                Some(at) if at != text.caret => {
                    text.place(at, true);
                    true
                }
                _ => false,
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
        Command::About => dialog::about(hwnd),
        // The spreadsheet's, and this pane has no answer to any of them.
        Command::GoTo
        | Command::Recalculate
        | Command::SheetAdd
        | Command::SheetRename
        | Command::SheetDelete
        | Command::SheetNext
        | Command::SheetPrevious => {}
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
fn draw_text_frame(dc: HDC, state: &Text) {
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
    text::draw::paint(
        dc,
        &text::draw::Frame {
            page: &state.page,
            theme: state.theme,
            faces: &faces,
            blocks: &blocks,
            selection: state.range(),
            caret: state.caret,
            caret_on: state.caret_on,
            status: &status,
            banner: state.banner.as_deref(),
            font_px: scale(FONT_PX, state.page.dpi).round() as i32,
            face: FACE,
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
