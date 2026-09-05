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
//!    `doc/windows-shell.md`'s decision 7, and W1 has no modal dialog yet — but the rule is
//!    written into [`with_state`] now rather than after the first `MessageBoxW` breaks it.

#![cfg(windows)]

use std::path::PathBuf;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, EndPaint, HDC, InvalidateRect, PAINTSTRUCT, UpdateWindow,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForWindow, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CREATESTRUCTW, CW_USEDEFAULT, CreateWindowExW, DefWindowProcW, DispatchMessageW, EN_KILLFOCUS,
    ES_AUTOHSCROLL, GWLP_USERDATA, GetMessageW, GetParent, GetWindowLongPtrW, GetWindowTextLengthW,
    GetWindowTextW, HMENU, IDC_ARROW, LoadCursorW, MSG, MoveWindow, PostQuitMessage,
    RegisterClassW, SB_BOTTOM, SB_HORZ, SB_LINEDOWN, SB_LINEUP, SB_PAGEDOWN, SB_PAGEUP,
    SB_THUMBPOSITION, SB_THUMBTRACK, SB_TOP, SB_VERT, SCROLLINFO, SCROLLINFO_MASK, SIF_PAGE,
    SIF_POS, SIF_RANGE, SPI_GETWHEELSCROLLLINES, SW_HIDE, SW_SHOW, SWP_FRAMECHANGED,
    SWP_NOACTIVATE, SWP_NOZORDER, SendMessageW, SetWindowLongPtrW, SetWindowPos, SetWindowTextW,
    ShowWindow, SystemParametersInfoW, TranslateMessage, WHEEL_DELTA, WM_APP, WM_COMMAND,
    WM_CREATE, WM_DESTROY, WM_DPICHANGED, WM_ERASEBKGND, WM_HSCROLL, WM_KEYDOWN, WM_LBUTTONDOWN,
    WM_LBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_NCCREATE, WM_NCDESTROY, WM_PAINT, WM_SETFONT,
    WM_SETTINGCHANGE, WM_SIZE, WM_VSCROLL, WNDCLASSW, WS_CHILD, WS_HSCROLL, WS_OVERLAPPEDWINDOW,
    WS_VSCROLL,
};
// Focus and mouse capture are Windows' input API rather than its window-management one, which
// is where its own metadata puts them.
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, ReleaseCapture, SetCapture, SetFocus, VK_CONTROL, VK_ESCAPE, VK_MENU, VK_RETURN,
    VK_SHIFT,
};
// `SetScrollInfo` lives in the Controls namespace in Windows' own metadata, which is where the
// scrollbar API has always been. It is *not* a Common Controls v6 class and needs no manifest —
// `doc/windows-shell.md`'s rejection of a v6 toolbar does not reach it.
use windows::Win32::UI::Controls::{EM_SETSEL, SetScrollInfo};
use windows::core::PCWSTR;

use crate::gdi::{self, BackBuffer, Dib, Font};
use crate::sheet::draw::{self, Frame};
use crate::sheet::geom::{GridGeom, Hit, MAX_COLS, MAX_ROWS, Sizes, scale};
use crate::sheet::keymap::{self, Selection};
use crate::sheet::status;
use crate::theme::{self, Mode, Theme};
use grind_sheet::Pos;

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

/// The child `EDIT` serving as the name box. One control, one id — `WM_COMMAND`'s low word is
/// how a notification says which child it came from.
const ID_NAME_BOX: usize = 1;

/// "The name box has finished", posted by the message loop.
///
/// Enter and Escape never reach a child `EDIT`'s parent on their own — the control swallows
/// them — so the pump in [`run`] watches for them and sends this instead. That is the standard
/// answer for a window with no dialog manager, and it is preferred here over subclassing the
/// control: a subclass means a second window procedure and a second lifetime to get right, and
/// `SetWindowSubclass` lives in `comctl32`, which this binary does not import and would rather
/// not start importing for one keystroke.
const WM_NAME_BOX_DONE: u32 = WM_APP + 1;

/// The frame `--render-to` draws, in pixels at 96 dpi.
///
/// Fixed rather than taken from the command line, because the output's whole purpose is to be
/// compared with another one: a size that could differ between two runs is a difference the
/// comparison would report as a drawing change.
const RENDER_W: i32 = 1280;
const RENDER_H: i32 = 800;

/// Everything the window owns. One per window, boxed, reached through `GWLP_USERDATA`.
struct State {
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
    /// The face the name box is set in. Owned here because a `WM_SETFONT` does not take a copy:
    /// the handle has to outlive every paint of the control, and be deleted after it.
    ui_font: Option<Font>,
}

/// What a drag started on, which decides what moving the mouse extends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Drag {
    Cells,
    Cols,
    Rows,
}

impl State {
    /// Rebuild the geometry for the current client size and DPI.
    ///
    /// Everything measured is rebuilt from the *document's* lengths rather than scaled from the
    /// last answer, so a window dragged between two monitors and back is pixel-identical to one
    /// that never moved — repeatedly scaling a scaled number is how that drifts.
    fn relayout(&mut self, width: f64, height: f64, dpi: u32) {
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

    fn title(&self) -> String {
        match &self.path {
            Some(path) => {
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.display().to_string());
                format!("{name} — {APP_NAME}")
            }
            None => APP_NAME.to_string(),
        }
    }
}

/// A document, read, with a window's worth of state around it and no window.
///
/// Shared by [`run`] and [`render`] on purpose: the render path is a *second caller* of every
/// answer the window uses, not a second set of answers, which is what makes a byte-identical
/// frame evidence about the real thing (`doc/windows-shell.md`, decision 5).
fn opened(path: Option<PathBuf>, theme: Theme) -> Result<State, String> {
    let app = grind_sheet::App::new();
    if let Some(path) = &path {
        app.open_file(path)
            .map_err(|error| format!("{}: {error}", path.display()))?;
    }
    Ok(State {
        app,
        path,
        sheet: 0,
        geom: GridGeom {
            strip_h: draw::STRIP_H,
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
        ui_font: None,
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
pub fn render(path: Option<PathBuf>, target: &std::path::Path) -> Result<(), String> {
    let mut state = opened(path, Theme::of(Mode::Light))?;
    state.relayout(f64::from(RENDER_W), f64::from(RENDER_H), 96);
    let dib = Dib::new(RENDER_W, RENDER_H).ok_or("could not make the drawing surface")?;
    draw_frame(dib.dc(), &state);
    std::fs::write(target, dib.bmp()).map_err(|error| format!("{}: {error}", target.display()))
}

/// Open a window on a document and pump messages until it closes.
///
/// `path` is `None` for a new, empty spreadsheet. The text pane is W5; a text document reaching
/// here is the caller's error to have made.
pub fn run(path: Option<PathBuf>) -> Result<(), String> {
    // Before *any* window exists, which is the whole requirement: per-monitor v2 cannot be set
    // once a window has been created, and asking for it late fails silently and leaves the
    // process system-DPI-aware — a window that is bitmap-stretched and blurry on a 150% monitor.
    // Best-effort because it also fails when a manifest already set an awareness, and that is
    // not a reason to refuse to start.
    // SAFETY: no arguments to outlive the call.
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }

    let state = Box::new(opened(path, theme::current())?);

    let class = gdi::wide(CLASS_NAME);
    let title = gdi::wide(&state.title());

    // SAFETY: the class name and title buffers outlive the calls; the boxed state is handed to
    // the window and taken back in `WM_NCDESTROY`. `CreateWindowExW` failing leaks nothing,
    // because the box is only released to the window once `WM_NCCREATE` has stored it — and
    // that message arrives before any failure path.
    unsafe {
        let instance = GetModuleHandleW(None).map_err(|error| format!("no module: {error}"))?;
        let wc = WNDCLASSW {
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
            // Enter and Escape inside a child `EDIT` never reach its parent: the control eats
            // them, and with no dialog manager there is nothing to turn them into a command.
            // Intercepting them in the pump is the standard answer, and it is cheaper and
            // shorter-lived than subclassing the control — see `WM_NAME_BOX_DONE`.
            //
            // The name box is this window's only child, so "the message went to a child of
            // ours" identifies it without the pump needing to reach the state at all.
            let vk = message.wParam.0 as u32;
            let finishes = vk == u32::from(VK_RETURN.0) || vk == u32::from(VK_ESCAPE.0);
            if message.message == WM_KEYDOWN
                && finishes
                && GetParent(message.hwnd).is_ok_and(|parent| parent == hwnd)
            {
                SendMessageW(
                    hwnd,
                    WM_NAME_BOX_DONE,
                    Some(WPARAM(usize::from(vk == u32::from(VK_RETURN.0)))),
                    Some(LPARAM(0)),
                );
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
unsafe fn with_state<T>(hwnd: HWND, f: impl FnOnce(&mut State) -> T) -> Option<T> {
    // SAFETY: the caller is a window procedure for `hwnd`, and the slot holds either null or
    // the pointer stored in `WM_NCCREATE`, which is valid until `WM_NCDESTROY` clears it.
    let raw = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut State;
    if raw.is_null() {
        return None;
    }
    // SAFETY: exclusive for the duration of this call. See the module comment's point 3.
    Some(f(unsafe { &mut *raw }))
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
                with_state(hwnd, |state| {
                    state.theme = theme::current();
                    theme::apply_title_bar(hwnd, state.theme);
                });
            }
            make_name_box(hwnd);
            resize(hwnd);
            LRESULT(0)
        }
        // Nothing to erase: the painter writes every pixel of the client area onto a back
        // buffer and blits it in one move, so letting the default erase first would show the
        // class brush for a frame. Answering non-zero is the documented way to say "done".
        WM_ERASEBKGND => LRESULT(1),
        WM_SIZE => {
            resize(hwnd);
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
        WM_KEYDOWN => match key_down(hwnd, wparam.0 as u32) {
            true => LRESULT(0),
            // SAFETY: a key this shell does not own, handed back with the arguments it came
            // with — which is what leaves Alt+F4 and the system menu working.
            false => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
        },
        // Enter or Escape in the name box, relayed by the pump in `run`.
        WM_NAME_BOX_DONE => {
            close_name_box(hwnd, wparam.0 != 0);
            LRESULT(0)
        }
        // The name box lost the focus without being finished — a click on the grid, or Alt+Tab.
        // Closing without committing is the same answer every address box gives.
        WM_COMMAND if (wparam.0 >> 16) as u32 == EN_KILLFOCUS => {
            close_name_box(hwnd, false);
            LRESULT(0)
        }
        WM_MOUSEWHEEL => {
            wheel(hwnd, wparam);
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
                with_state(hwnd, |state| {
                    state.theme = theme::current();
                    theme::apply_title_bar(hwnd, state.theme);
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
            resize(hwnd);
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
                let raw = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut State;
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

/// Recompute the geometry and the scrollbars for the current client area, then repaint.
fn resize(hwnd: HWND) {
    let rect = gdi::client_rect(hwnd);
    // SAFETY: `hwnd` is this window's; `GetDpiForWindow` needs nothing else.
    let dpi = unsafe { GetDpiForWindow(hwnd) }.max(96);
    // SAFETY: no nested loop inside.
    let moved = unsafe {
        with_state(hwnd, |state| {
            state.relayout(
                f64::from(rect.right - rect.left),
                f64::from(rect.bottom - rect.top),
                dpi,
            );
            // A window that grew or changed monitor may now show the active cell where it did
            // not, and — more to the point — may not show it any more.
            state.reveal();
            sync_scrollbars(hwnd, state);
            state
                .name_box_open
                .then_some((state.name_box, state.geom.name_box_rect()))
        })
    };
    // SAFETY: the borrow above is released. `MoveWindow` on a visible child repaints it, which
    // is a message dispatch and therefore belongs out here.
    unsafe {
        if let Some(Some((edit, rect))) = moved {
            let _ = MoveWindow(
                edit,
                rect.x.round() as i32,
                rect.y.round() as i32,
                rect.w.round() as i32,
                rect.h.round() as i32,
                true,
            );
        }
        let _ = InvalidateRect(Some(hwnd), None, false);
    }
}

/// Create the child `EDIT` that becomes the name box when somebody types in it.
///
/// Hidden from birth and shown on demand. Decision 2 draws the line this sits on: a control that
/// holds a *keystroke* is a widget, and a control that holds the *document* is a second model —
/// this one holds an address on its way to `status::locate` and nothing else.
fn make_name_box(hwnd: HWND) {
    let class = gdi::wide("EDIT");
    // SAFETY: the class name outlives the call; the control is destroyed with its parent, and
    // the font it is given is owned by the state, which outlives both.
    unsafe {
        let Ok(edit) = CreateWindowExW(
            Default::default(),
            PCWSTR(class.as_ptr()),
            PCWSTR::null(),
            windows::Win32::UI::WindowsAndMessaging::WINDOW_STYLE(ES_AUTOHSCROLL as u32) | WS_CHILD,
            0,
            0,
            0,
            0,
            Some(hwnd),
            Some(HMENU(ID_NAME_BOX as *mut std::ffi::c_void)),
            None,
            None,
        ) else {
            return;
        };
        let dpi = GetDpiForWindow(hwnd).max(96);
        let font = Font::new(FACE, scale(FONT_PX, dpi).round() as i32, false);
        SendMessageW(
            edit,
            WM_SETFONT,
            Some(WPARAM(font.handle().0 as usize)),
            Some(LPARAM(1)),
        );
        with_state(hwnd, |state| {
            state.name_box = edit;
            // Kept alive here: `WM_SETFONT` does not copy the handle, so deleting the font
            // while the control still refers to it is the leak-shaped bug in reverse.
            state.ui_font = Some(font);
        });
    }
}

/// Tell Windows where the thumbs are and how big they are.
///
/// The range is in **tracks, not pixels** — see `GridGeom`'s comment: a `SCROLLINFO` is `i32`
/// and this sheet is twenty million pixels tall, but only a million rows.
fn sync_scrollbars(hwnd: HWND, state: &State) {
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
    let code = (wparam.0 & 0xffff) as i32;
    let thumb = ((wparam.0 >> 16) & 0xffff) as u32;
    // SAFETY: no nested loop inside.
    unsafe {
        with_state(hwnd, |state| {
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
    // Zero means "do not scroll", which is a real setting. `WHEEL_PAGESCROLL` (0xFFFFFFFF)
    // means a screenful, and is answered with the page rather than with 4 294 967 295 rows.
    // SAFETY: no nested loop inside.
    unsafe {
        with_state(hwnd, |state| {
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
    // SAFETY: no nested loop inside.
    let moved = unsafe {
        with_state(hwnd, |state| {
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

/// One keystroke. `false` means this shell does not own the key, and the caller hands it back to
/// `DefWindowProc` so that Alt+F4 and the system menu keep working.
fn key_down(hwnd: HWND, vk: u32) -> bool {
    let Some(action) = keymap::action_for(keymap::key_for(vk), mods()) else {
        return false;
    };
    // Opening the name box moves the focus and shows a window, so it happens with nothing
    // borrowed — decision 7's rule, applied to a control rather than to a dialog.
    if action == keymap::Action::GoTo {
        open_name_box(hwnd);
        return true;
    }
    // SAFETY: no nested loop inside.
    unsafe {
        with_state(hwnd, |state| {
            state.act(action);
            sync_scrollbars(hwnd, state);
        });
        let _ = InvalidateRect(Some(hwnd), None, false);
    }
    true
}

/// A press of the left button: what it selects, and what dragging will extend.
fn button_down(hwnd: HWND, lparam: LPARAM) {
    let (x, y) = point(lparam);
    let extend = mods().shift;
    // The name box is drawn chrome until it is clicked, at which point it becomes a control.
    // Deciding that needs the geometry, so it is read here and acted on below the borrow.
    // SAFETY: no nested loop inside.
    let name_box = unsafe {
        with_state(hwnd, |state| {
            let hit = state.geom.hit(x, y);
            if hit == Hit::Chrome {
                return state.geom.name_box_rect().contains(x, y);
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
            false
        })
    };
    // SAFETY: the borrow is released. `SetCapture` and `SetFocus` both send messages
    // synchronously, which is exactly why they are out here.
    unsafe {
        if name_box == Some(true) {
            open_name_box(hwnd);
            return;
        }
        // Capture so that a drag that leaves the window still reports where it went, and take
        // the focus back off the name box if it had it.
        let _ = SetCapture(hwnd);
        let _ = SetFocus(Some(hwnd));
        let _ = InvalidateRect(Some(hwnd), None, false);
    }
}

fn button_up(hwnd: HWND) {
    // SAFETY: no nested loop inside.
    unsafe {
        with_state(hwnd, |state| state.drag = None);
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
    // SAFETY: one borrow, for the two answers, released before anything is shown.
    let Some((edit, rect, text)) = (unsafe {
        with_state(hwnd, |state| {
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
    // SAFETY: the control is this window's child and outlives the call; the text buffer is a
    // NUL-terminated local, which is what `SetWindowTextW`'s `PCWSTR` wants.
    unsafe {
        let _ = MoveWindow(
            edit,
            rect.x.round() as i32,
            rect.y.round() as i32,
            rect.w.round() as i32,
            rect.h.round() as i32,
            false,
        );
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
        with_state(hwnd, |state| {
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
        with_state(hwnd, |state| {
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

/// Everything one frame needs, drawn onto a device context.
///
/// Takes an `HDC` and the state and nothing about the window, which is what makes
/// [`render`] a second *caller* rather than a second drawing path.
fn draw_frame(dc: HDC, state: &State) {
    let rows = state.geom.visible_rows();
    let cols = state.geom.visible_cols();
    let viewport = state
        .app
        .get_viewport(state.sheet, rows, cols)
        .unwrap_or_else(|_| {
            state
                .app
                .get_viewport(state.sheet, 0..0, 0..0)
                .expect("an empty rectangle of the first sheet always reads")
        });
    let status = status::status_line(&state.app, state.sheet, state.selection);
    let name = status::name_box_text(&state.app, state.sheet, state.selection);
    draw::paint(
        dc,
        &Frame {
            geom: &state.geom,
            theme: state.theme,
            viewport: &viewport,
            status: &status,
            name: &name,
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
            with_state(hwnd, |state| {
                buffer.clear(state.theme.background);
                draw_frame(buffer.dc(), state);
            });
            buffer.present(dc);
        }
        let _ = EndPaint(hwnd, &ps);
    }
}
