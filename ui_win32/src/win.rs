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
    CREATESTRUCTW, CW_USEDEFAULT, CreateWindowExW, DefWindowProcW, DispatchMessageW, GWLP_USERDATA,
    GetMessageW, GetWindowLongPtrW, HMENU, IDC_ARROW, LoadCursorW, MSG, PostQuitMessage,
    RegisterClassW, SB_BOTTOM, SB_HORZ, SB_LINEDOWN, SB_LINEUP, SB_PAGEDOWN, SB_PAGEUP,
    SB_THUMBPOSITION, SB_THUMBTRACK, SB_TOP, SB_VERT, SCROLLINFO, SCROLLINFO_MASK, SIF_PAGE,
    SIF_POS, SIF_RANGE, SPI_GETWHEELSCROLLLINES, SW_SHOW, SWP_FRAMECHANGED, SWP_NOACTIVATE,
    SWP_NOZORDER, SetWindowLongPtrW, SetWindowPos, ShowWindow, SystemParametersInfoW,
    TranslateMessage, WHEEL_DELTA, WM_CREATE, WM_DESTROY, WM_DPICHANGED, WM_ERASEBKGND, WM_HSCROLL,
    WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_NCCREATE, WM_NCDESTROY, WM_PAINT, WM_SETTINGCHANGE, WM_SIZE,
    WM_VSCROLL, WNDCLASSW, WS_HSCROLL, WS_OVERLAPPEDWINDOW, WS_VSCROLL,
};
// `SetScrollInfo` lives in the Controls namespace in Windows' own metadata, which is where the
// scrollbar API has always been. It is *not* a Common Controls v6 class and needs no manifest —
// `doc/windows-shell.md`'s rejection of a v6 toolbar does not reach it.
use windows::Win32::UI::Controls::SetScrollInfo;
use windows::core::PCWSTR;

use crate::gdi::{self, BackBuffer};
use crate::sheet::draw::{self, Frame};
use crate::sheet::geom::{GridGeom, Hit, MAX_COLS, MAX_ROWS, Sizes, scale};
use crate::theme::{self, Theme};

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

/// Everything the window owns. One per window, boxed, reached through `GWLP_USERDATA`.
struct State {
    app: grind_sheet::App,
    path: Option<PathBuf>,
    sheet: usize,
    geom: GridGeom,
    theme: Theme,
    /// What the pointer is over, or `None` when it is outside the grid.
    ///
    /// W1 has no selection — that is W2 — so this is what the status bar reports, and it is
    /// there for a reason beyond being informative: it is the only thing in this milestone that
    /// exercises `geom::hit`, so a hit-test that disagrees with where cells are drawn is
    /// visible by moving the mouse rather than only by a unit test.
    pointer: Option<grind_sheet::Pos>,
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

    /// What the status bar says: the sheet, the extent of what is in it, and where the view is.
    fn status(&self) -> String {
        let name = self
            .app
            .sheet_name(self.sheet)
            .unwrap_or_else(|_| "Sheet1".into());
        let (rows, cols) = self.app.used_extent(self.sheet).unwrap_or((0, 0));
        let sheets = self.app.sheet_count();
        let at = grind_sheet::a1::format(
            None,
            self.pointer
                .unwrap_or_else(|| grind_sheet::Pos::new(self.geom.first_row, self.geom.first_col)),
        );
        format!(
            "{name}  ({} of {sheets})   {rows} \u{00d7} {cols} used   {at}",
            self.sheet + 1
        )
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

    let app = grind_sheet::App::new();
    if let Some(path) = &path {
        app.open_file(path)
            .map_err(|error| format!("{}: {error}", path.display()))?;
    }
    let state = Box::new(State {
        app,
        path,
        sheet: 0,
        geom: GridGeom {
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
        theme: theme::current(),
        pointer: None,
    });

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
    unsafe {
        with_state(hwnd, |state| {
            state.relayout(
                f64::from(rect.right - rect.left),
                f64::from(rect.bottom - rect.top),
                dpi,
            );
            sync_scrollbars(hwnd, state);
        });
        let _ = InvalidateRect(Some(hwnd), None, false);
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

/// Track which cell the pointer is over, and repaint the status bar when it changes.
///
/// Only the status bar is invalidated rather than the window, because a mouse move arrives on
/// every pixel of travel and repainting a screenful of cells for each one is how a grid comes to
/// feel slow on a machine nobody testing it has.
fn mouse_move(hwnd: HWND, lparam: LPARAM) {
    // The position is packed as two *signed* 16-bit numbers, and the sign matters: dragging
    // above or left of the client area gives a negative coordinate rather than a huge one.
    let x = f64::from((lparam.0 & 0xffff) as i16);
    let y = f64::from(((lparam.0 >> 16) & 0xffff) as i16);
    // SAFETY: no nested loop inside.
    unsafe {
        with_state(hwnd, |state| {
            let was = state.pointer;
            state.pointer = match state.geom.hit(x, y) {
                Hit::Cell { row, col } => Some(grind_sheet::Pos::new(row, col)),
                _ => None,
            };
            if state.pointer != was {
                let status = state.geom.status_rect();
                let rect = windows::Win32::Foundation::RECT {
                    left: status.x.round() as i32,
                    top: status.y.round() as i32,
                    right: (status.x + status.w).round() as i32,
                    bottom: (status.y + status.h).round() as i32,
                };
                let _ = InvalidateRect(Some(hwnd), Some(&rect), false);
            }
        });
    }
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
                let status = state.status();
                draw::paint(
                    buffer.dc(),
                    &Frame {
                        geom: &state.geom,
                        theme: state.theme,
                        viewport: &viewport,
                        status: &status,
                        font_px: scale(FONT_PX, state.geom.dpi).round() as i32,
                        face: FACE,
                    },
                );
            });
            buffer.present(dc);
        }
        let _ = EndPaint(hwnd, &ps);
    }
}
