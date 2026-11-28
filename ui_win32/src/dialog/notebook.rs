// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The tape's body in a modal (`crate::tape`, `doc/calc.md` §7): its rows drawn, one rich edit
//! over the caret's row that colours what is typed, the offers under the rows. Every decision is
//! `grind_sheet::TapeEditor`'s; after each one [`sync`] puts the edit where the caret is and
//! fills it.

use grind_core::utf16;
use grind_sheet::{Notebook, Row, Tape, TapeEditor, TapeSpan};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{HDC, InvalidateRect};
use windows::Win32::System::LibraryLoader::LoadLibraryW;
use windows::Win32::UI::Controls::NMHDR;
use windows::Win32::UI::Controls::RichEdit::{
    CFM_COLOR, CHARFORMATW, CHARRANGE, EM_EXSETSEL, EM_SETBKGNDCOLOR, EM_SETCHARFORMAT,
    EM_SETEVENTMASK, EN_SELCHANGE, ENM_CHANGE, ENM_SELCHANGE, SCF_ALL, SCF_SELECTION,
};
use windows::Win32::UI::Controls::{EM_GETSEL, EM_SETSEL};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    EnableWindow, GetKeyState, SetFocus, VK_CONTROL, VK_SHIFT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EN_CHANGE, ES_AUTOHSCROLL, SWP_NOACTIVATE, SWP_NOZORDER, SendMessageW, SetWindowPos,
    SetWindowTextW, WINDOW_STYLE, WM_SETREDRAW, WS_TABSTOP,
};
use windows::core::{PCWSTR, w};

use super::{
    Body, ID_EDIT, IDCANCEL, Modal, Role, Spec, rect_of, run, text_box, window_text, with_modal,
};
use crate::gdi::{self, Across, Selected};
use crate::modal;
use crate::sheet::geom::Rect;
use crate::tape::{self as portable, Layout};
use crate::theme::{Rgb, Theme};

const ID_UNDO: i32 = 220;
const ID_REDO: i32 = 221;
const ID_OPEN: i32 = 222;
const ID_LOAD: i32 = 223;

/// What the modal holds while the tape is up.
pub(super) struct TapeBody {
    editor: TapeEditor,
    /// The first row shown.
    top: usize,
    /// Whether the edit is a rich edit, which colours; a plain one where Msftedit is missing.
    rich: bool,
    /// Set while [`sync`] fills the edit, so its own notifications are not typing.
    syncing: bool,
}

/// The tape until the modal closes: what it holds then, and whether it closed on *Open as
/// Spreadsheet*.
pub fn scratch(owner: HWND, tape: Tape) -> (Tape, bool) {
    let kept = tape.clone();
    let body = TapeBody {
        editor: TapeEditor::new(tape),
        top: 0,
        rich: false,
        syncing: false,
    };
    run(
        Spec {
            owner,
            title: "Scratch Calculator",
            body: Body::Tape(Box::new(body)),
            buttons: vec![
                (ID_UNDO, "Undo", Role::Leading),
                (ID_REDO, "Redo", Role::Leading),
                (ID_LOAD, "Open Tape…", Role::Leading),
                (IDCANCEL.0, "Close", Role::Answer),
                (ID_OPEN, "Open as Spreadsheet", Role::Default),
            ],
        },
        |modal| {
            let opened = modal.answer == Some(ID_OPEN);
            match std::mem::replace(&mut modal.body, Body::Message(String::new())) {
                Body::Tape(body) => Some((body.editor.into_tape(), opened)),
                _ => None,
            }
        },
    )
    .flatten()
    .unwrap_or((kept, false))
}

/// The edit for the caret's row: a rich edit where Msftedit loads, else a plain one.
pub(super) fn control(
    popup: HWND,
    make: &dyn Fn(&str, &str, WINDOW_STYLE, usize, RECT) -> HWND,
    theme: Theme,
) -> HWND {
    let style = WINDOW_STYLE(ES_AUTOHSCROLL as u32) | WS_TABSTOP;
    // SAFETY: loading a system library by name; it stays loaded for the process.
    let loaded = unsafe { LoadLibraryW(w!("Msftedit.dll")) }.is_ok();
    let rich = if loaded {
        make("RICHEDIT50W", "", style, ID_EDIT, RECT::default())
    } else {
        HWND::default()
    };
    let (edit, is_rich) = if rich.is_invalid() {
        (make("EDIT", "", style, ID_EDIT, RECT::default()), false)
    } else {
        (rich, true)
    };
    if is_rich {
        // SAFETY: messages to the control just made.
        unsafe {
            SendMessageW(
                edit,
                EM_SETEVENTMASK,
                Some(WPARAM(0)),
                Some(LPARAM((ENM_CHANGE | ENM_SELCHANGE) as isize)),
            );
            SendMessageW(
                edit,
                EM_SETBKGNDCOLOR,
                Some(WPARAM(0)),
                Some(LPARAM(theme.background.colorref() as isize)),
            );
        }
    }
    // SAFETY: one borrow, no dispatch.
    unsafe {
        with_modal(popup, |modal| {
            if let Body::Tape(body) = &mut modal.body {
                body.rich = is_rich;
            }
        });
    }
    edit
}

/// Put the edit over the caret's row, scrolled into view, holding the editor's text with the
/// caret where the editor has it; the buttons and the rows to match.
pub(super) fn sync(popup: HWND) {
    // SAFETY: one borrow to read, the edit filled with it released, one more to finish.
    unsafe {
        let client = gdi::client_rect(popup);
        let Some(Some(state)) = with_modal(popup, |modal| {
            let dpi = modal.dpi;
            let frame = modal::frame(
                f64::from(client.right),
                f64::from(client.bottom),
                dpi,
                !modal.title.is_empty(),
            );
            let layout = Layout::of(frame.body, dpi);
            let find = |id| {
                modal
                    .buttons
                    .iter()
                    .find(|button| button.id == id)
                    .map_or(HWND::default(), |button| button.hwnd)
            };
            let (undo, redo) = (find(ID_UNDO), find(ID_REDO));
            let line = modal.px(crate::theme::text::BODY * 1.4);
            let inset = modal.px(11.0);
            let (control, theme) = (modal.control, modal.theme);
            let Body::Tape(body) = &mut modal.body else {
                return None;
            };
            let at = body
                .editor
                .rows()
                .iter()
                .position(|row| *row == body.editor.row())
                .unwrap_or(0);
            body.top = portable::scrolled(body.top, at, layout.visible());
            let well = rect_of(layout.text(layout.row(at - body.top)));
            let top = well.top + (well.bottom - well.top - line) / 2;
            let place = RECT {
                left: well.left + inset,
                top,
                right: well.right - inset,
                bottom: top + line,
            };
            let text = body.editor.text().to_owned();
            let caret = utf16::units_before(&text, body.editor.caret()) as i32;
            let (_, spans) = body.editor.shown(body.editor.row());
            let tape = body.editor.tape();
            let (can_undo, can_redo) = (tape.can_undo(), tape.can_redo());
            body.syncing = true;
            let rich = body.rich;
            modal.well = well;
            Some(Synced {
                control,
                place,
                text,
                caret,
                spans,
                rich,
                theme,
                buttons: [(undo, can_undo), (redo, can_redo)],
            })
        }) else {
            return;
        };
        let place = state.place;
        let _ = SetWindowPos(
            state.control,
            None,
            place.left,
            place.top,
            place.right - place.left,
            place.bottom - place.top,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
        if window_text(state.control) != state.text {
            let wide = gdi::wide(&state.text);
            let _ = SetWindowTextW(state.control, PCWSTR(wide.as_ptr()));
        }
        let caret = CHARRANGE {
            cpMin: state.caret,
            cpMax: state.caret,
        };
        if state.rich {
            colour(state.control, &state.text, &state.spans, state.theme, caret);
        } else {
            SendMessageW(
                state.control,
                EM_SETSEL,
                Some(WPARAM(caret.cpMin as usize)),
                Some(LPARAM(caret.cpMax as isize)),
            );
        }
        for (button, can) in state.buttons {
            let _ = EnableWindow(button, can);
            let _ = InvalidateRect(Some(button), None, false);
        }
        with_modal(popup, |modal| {
            if let Body::Tape(body) = &mut modal.body {
                body.syncing = false;
            }
        });
        let _ = InvalidateRect(Some(popup), None, false);
    }
}

struct Synced {
    control: HWND,
    place: RECT,
    text: String,
    caret: i32,
    spans: Vec<TapeSpan>,
    rich: bool,
    theme: Theme,
    buttons: [(HWND, bool); 2],
}

/// Colour the rich edit's words, then put the selection back to `keep`. Drawn once at the end.
unsafe fn colour(control: HWND, text: &str, spans: &[TapeSpan], theme: Theme, keep: CHARRANGE) {
    let format = |ink: Rgb| CHARFORMATW {
        cbSize: size_of::<CHARFORMATW>() as u32,
        dwMask: CFM_COLOR,
        crTextColor: COLORREF(ink.colorref()),
        ..Default::default()
    };
    let units = |byte: usize| utf16::units_before(text, byte) as i32;
    // SAFETY: messages to a live control, each handed a local that outlives it.
    unsafe {
        SendMessageW(control, WM_SETREDRAW, Some(WPARAM(0)), None);
        let all = format(theme.text);
        SendMessageW(
            control,
            EM_SETCHARFORMAT,
            Some(WPARAM(SCF_ALL as usize)),
            Some(LPARAM(std::ptr::from_ref(&all) as isize)),
        );
        for span in spans {
            let Some(ink) = portable::ink(span.ink, &theme) else {
                continue;
            };
            let range = CHARRANGE {
                cpMin: units(span.range.start),
                cpMax: units(span.range.end),
            };
            SendMessageW(
                control,
                EM_EXSETSEL,
                Some(WPARAM(0)),
                Some(LPARAM(std::ptr::from_ref(&range) as isize)),
            );
            let run = format(ink);
            SendMessageW(
                control,
                EM_SETCHARFORMAT,
                Some(WPARAM(SCF_SELECTION as usize)),
                Some(LPARAM(std::ptr::from_ref(&run) as isize)),
            );
        }
        SendMessageW(
            control,
            EM_EXSETSEL,
            Some(WPARAM(0)),
            Some(LPARAM(std::ptr::from_ref(&keep) as isize)),
        );
        SendMessageW(control, WM_SETREDRAW, Some(WPARAM(1)), None);
        let _ = InvalidateRect(Some(control), None, true);
    }
}

/// The edit's text or caret changed under somebody's hand: the editor told, the words coloured
/// again, the offers redrawn.
fn typed(popup: HWND) {
    // SAFETY: borrows released around every message to the edit.
    unsafe {
        let Some(Some((control, rich))) = with_modal(popup, |modal| match &modal.body {
            Body::Tape(body) if !body.syncing => Some((modal.control, body.rich)),
            _ => None,
        }) else {
            return;
        };
        let text = window_text(control);
        let (mut start, mut end) = (0u32, 0u32);
        SendMessageW(
            control,
            EM_GETSEL,
            Some(WPARAM(std::ptr::from_mut(&mut start) as usize)),
            Some(LPARAM(std::ptr::from_mut(&mut end) as isize)),
        );
        let caret = utf16::byte_of(&text, end as usize);
        let Some(Some((spans, theme))) = with_modal(popup, |modal| {
            let theme = modal.theme;
            let Body::Tape(body) = &mut modal.body else {
                return None;
            };
            body.editor.typed(&text, caret);
            body.syncing = true;
            Some((body.editor.shown(body.editor.row()).1, theme))
        }) else {
            return;
        };
        if rich {
            let keep = CHARRANGE {
                cpMin: start as i32,
                cpMax: end as i32,
            };
            colour(control, &text, &spans, theme, keep);
        }
        with_modal(popup, |modal| {
            if let Body::Tape(body) = &mut modal.body {
                body.syncing = false;
            }
        });
        let _ = InvalidateRect(Some(popup), None, false);
    }
}

/// A key pressed in the edit, before the dialog manager sees it. `true` when the editor took it.
pub(super) fn key(popup: HWND, target: HWND, vk: u16) -> bool {
    // SAFETY: one borrow at a time, none across a message.
    unsafe {
        let Some(Some(control)) = with_modal(popup, |modal| {
            matches!(modal.body, Body::Tape(_)).then_some(modal.control)
        }) else {
            return false;
        };
        if target != control {
            return false;
        }
        let held = |vk: u16| GetKeyState(i32::from(vk)) < 0;
        let shift = held(VK_SHIFT.0);
        typed(popup);
        let took = with_modal(popup, |modal| {
            let Body::Tape(body) = &mut modal.body else {
                return false;
            };
            // Ctrl+Z inside the tape is the tape's undo, as everywhere else in the shell.
            if held(VK_CONTROL.0) {
                match (vk, shift) {
                    (0x5a, false) => body.editor.undo(),
                    (0x5a, true) | (0x59, _) => body.editor.redo(),
                    _ => return false,
                }
                return true;
            }
            portable::key(vk, shift).is_some_and(|key| body.editor.key(key))
        })
        .unwrap_or(false);
        if took {
            sync(popup);
        }
        took
    }
}

/// A click on a row moves the caret there.
pub(super) fn click(popup: HWND, lparam: LPARAM) -> bool {
    let x = f64::from(lparam.0 as i16);
    let y = f64::from((lparam.0 >> 16) as i16);
    let client = gdi::client_rect(popup);
    // SAFETY: one borrow, no dispatch inside it.
    let Some(Some(control)) = (unsafe {
        with_modal(popup, |modal| {
            let frame = modal::frame(
                f64::from(client.right),
                f64::from(client.bottom),
                modal.dpi,
                !modal.title.is_empty(),
            );
            let layout = Layout::of(frame.body, modal.dpi);
            let control = modal.control;
            let Body::Tape(body) = &mut modal.body else {
                return None;
            };
            let shown = layout.hit(x, y)?;
            let row = *body.editor.rows().get(body.top + shown)?;
            body.editor.click(row);
            Some(control)
        })
    }) else {
        return false;
    };
    sync(popup);
    // SAFETY: focusing a live control.
    unsafe {
        let _ = SetFocus(Some(control));
    }
    true
}

/// The tape's share of `WM_COMMAND`: the edit's changes and the Undo and Redo buttons. `true`
/// when it was the tape's.
pub(super) fn command(popup: HWND, id: i32, code: u32) -> bool {
    // SAFETY: one borrow at a time.
    unsafe {
        if with_modal(popup, |modal| matches!(modal.body, Body::Tape(_))) != Some(true) {
            return false;
        }
        if id as usize == ID_EDIT && code == EN_CHANGE {
            typed(popup);
            return true;
        }
        if id == ID_LOAD {
            // The picker is its own modal over this one; the tape is touched only once it answers.
            if let Some(path) = super::open_tape_path(popup) {
                let control = with_modal(popup, |modal| {
                    if let Body::Tape(body) = &mut modal.body {
                        body.editor.open_file(&path);
                    }
                    modal.control
                });
                sync(popup);
                if let Some(control) = control {
                    let _ = SetFocus(Some(control));
                }
            }
            return true;
        }
        if id != ID_UNDO && id != ID_REDO {
            return false;
        }
        let control = with_modal(popup, |modal| {
            if let Body::Tape(body) = &mut modal.body {
                if id == ID_UNDO {
                    body.editor.undo();
                } else {
                    body.editor.redo();
                }
            }
            modal.control
        });
        sync(popup);
        if let Some(control) = control {
            let _ = SetFocus(Some(control));
        }
        true
    }
}

/// A rich edit's caret moved: the offers follow it.
pub(super) fn notify(popup: HWND, lparam: LPARAM) -> bool {
    // SAFETY: `lparam` is the notification's `NMHDR`.
    let code = unsafe { (*(lparam.0 as *const NMHDR)).code };
    if code != EN_SELCHANGE {
        return false;
    }
    typed(popup);
    true
}

/// The rows, the offers and the note.
pub(super) fn paint(modal: &Modal, out: HDC, body: Rect) {
    let Body::Tape(tape) = &modal.body else {
        return;
    };
    let theme = modal.theme;
    let layout = Layout::of(body, modal.dpi);
    let editor = &tape.editor;
    let lines = editor.tape().lines();
    let inset = modal.px(11.0);
    let _font = Selected::font(out, &modal.body_font);
    let rows = editor.rows();
    for (shown, row) in rows
        .iter()
        .skip(tape.top)
        .take(layout.visible())
        .enumerate()
    {
        let r = layout.row(shown);
        let (left, _, right, bottom) = r.edges();
        gdi::fill(out, left, bottom - 1, right, bottom, theme.divider);
        let label = Notebook::label(*row, editor.tape());
        gdi::line(
            out,
            &label,
            rect_of(layout.handle(r)),
            Across::Left,
            theme.text_secondary,
        );
        let well = rect_of(layout.text(r));
        if *row == editor.row() {
            text_box(modal, out, well, modal.focused);
        } else {
            let (text, spans) = editor.shown(*row);
            runs(out, &text, &spans, &theme, well, inset);
        }
        if let Row::Line(at) = row
            && let Some(line) = lines.get(*at)
        {
            let ink = if line.error {
                theme.misspelt
            } else {
                theme.text
            };
            gdi::line(
                out,
                &line.value,
                rect_of(layout.value(r)),
                Across::Right,
                ink,
            );
        }
    }

    let band = editor.band();
    let band_rect = rect_of(layout.band);
    if band.is_empty() {
        gdi::line(
            out,
            "Enter: next line   \u{b7}   \u{2191}\u{2193}: move   \u{b7}   Tab: complete   \u{b7}   Ctrl+Z: undo",
            band_rect,
            Across::Left,
            theme.text_tertiary,
        );
    } else {
        let mut x = band_rect.left;
        for piece in band {
            if x >= band_rect.right {
                break;
            }
            let ink = match piece.ink {
                grind_sheet::formula::assist::Ink::Plain => theme.text,
                grind_sheet::formula::assist::Ink::Muted => theme.text_tertiary,
                grind_sheet::formula::assist::Ink::Strong => theme.accent,
            };
            let width = gdi::text_width(out, &piece.text);
            gdi::line(
                out,
                &piece.text,
                RECT {
                    left: x,
                    right: (x + width + 2).min(band_rect.right),
                    ..band_rect
                },
                Across::Left,
                ink,
            );
            x += width;
        }
    }
    let (note, ink) = if editor.tape().is_empty() && editor.note().is_empty() {
        (
            "A formula without its =, as in a cell. #2 reads line 2; vat = 0.19 names a line.",
            theme.text_secondary,
        )
    } else {
        (editor.note(), theme.misspelt)
    };
    gdi::line(out, note, rect_of(layout.note), Across::Left, ink);
}

/// A line's words in their colours, left to right inside `well`, cut at its edge.
fn runs(out: HDC, text: &str, spans: &[TapeSpan], theme: &Theme, well: RECT, inset: i32) {
    let mut pieces: Vec<(&str, Rgb)> = Vec::new();
    let mut at = 0;
    for span in spans {
        if let (Some(gap), Some(word)) =
            (text.get(at..span.range.start), text.get(span.range.clone()))
        {
            pieces.push((gap, theme.text));
            pieces.push((word, portable::ink(span.ink, theme).unwrap_or(theme.text)));
            at = span.range.end;
        }
    }
    pieces.push((text.get(at..).unwrap_or_default(), theme.text));
    let edge = well.right - inset;
    let mut x = well.left + inset;
    for (piece, ink) in pieces {
        if piece.is_empty() {
            continue;
        }
        if x >= edge {
            return;
        }
        let width = gdi::text_width(out, piece);
        gdi::line(
            out,
            piece,
            RECT {
                left: x,
                right: (x + width + 2).min(edge),
                ..well
            },
            Across::Left,
            ink,
        );
        x += width;
    }
}
