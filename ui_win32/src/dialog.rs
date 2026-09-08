// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Every modal this shell opens: the file dialogs, the close question, and one text prompt.
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
use windows::Win32::UI::WindowsAndMessaging::{
    IDCANCEL, IDNO, IDOK, IDYES, MB_DEFBUTTON2, MB_ICONERROR, MB_ICONINFORMATION, MB_ICONWARNING,
    MB_OK, MB_YESNO, MB_YESNOCANCEL, MESSAGEBOX_STYLE, MessageBoxW,
};
use windows::core::PCWSTR;

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

/// Say something and wait to be dismissed.
fn say(owner: HWND, title: &str, text: &str, style: MESSAGEBOX_STYLE) -> i32 {
    let text = gdi::wide(text);
    let title = gdi::wide(title);
    // SAFETY: both buffers are NUL-terminated locals that outlive the call. **This runs a
    // nested message loop** — see the module comment.
    unsafe {
        MessageBoxW(
            Some(owner),
            PCWSTR(text.as_ptr()),
            PCWSTR(title.as_ptr()),
            style,
        )
        .0
    }
}

pub fn error(owner: HWND, text: &str) {
    say(owner, "Grind", text, MB_OK | MB_ICONERROR);
}

/// The Help menu's one item: what build this is, over `MessageBoxW` rather than a window of its
/// own — the same choice `error`/`confirm` made, and `About Grind` asks nothing back.
///
/// `grind_core::build_info::describe` is the one place this fact is formatted, so this window's
/// About box reads the same commit/tree/date every other shell's does.
pub fn about(owner: HWND) {
    let text = format!(
        "An ODF-native spreadsheet.\n\n{}\n\nhttps://github.com/fwilhe2/grind",
        grind_core::build_info::describe("grind-win32", env!("CARGO_PKG_VERSION"))
    );
    say(owner, "About Grind", &text, MB_OK | MB_ICONINFORMATION);
}

/// The three-button close question.
///
/// Save is the default answer, which is Windows' convention and the safe one: a `MB_YESNOCANCEL`
/// box makes the first button the default, and the first button is Yes.
pub fn confirm_close(owner: HWND, name: &str) -> Answer {
    let text = format!("Save changes to {name} before closing?");
    match say(owner, "Grind", &text, MB_YESNOCANCEL | MB_ICONWARNING) {
        v if v == IDYES.0 => Answer::Save,
        v if v == IDNO.0 => Answer::Discard,
        _ => Answer::Cancel,
    }
}

/// Yes or no, with **No** as the default — deleting a sheet is the one destructive thing this
/// milestone can do, it takes a document's data with it, and a dialog whose default answer is
/// the destructive one is a dialog that deletes things when somebody presses Enter twice.
///
/// `MB_DEFBUTTON2` is what says that: `MB_YESNO` alone makes the *first* button the default,
/// which is the right answer for [`confirm_close`] (saving is the safe answer there) and the
/// wrong one here.
pub fn confirm(owner: HWND, text: &str) -> bool {
    say(
        owner,
        "Grind",
        text,
        MB_YESNO | MB_DEFBUTTON2 | MB_ICONWARNING,
    ) == IDYES.0
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
    vec![(
        gdi::wide("OpenDocument Spreadsheet or Text"),
        gdi::wide("*.fods;*.ods;*.fodt;*.odt;*.grind"),
    )]
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
// The text prompt
// ---------------------------------------------------------------------------

use std::sync::atomic::{AtomicBool, Ordering};

use windows::Win32::Graphics::Gdi::{
    COLOR_BTNFACE, COLOR_BTNTEXT, GetSysColor, GetSysColorBrush, HBRUSH, HDC, SetBkMode,
    SetTextColor, TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::EM_SETSEL;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::{EnableWindow, SetActiveWindow, SetFocus};
use windows::Win32::UI::WindowsAndMessaging::{
    BS_DEFPUSHBUTTON, CREATESTRUCTW, CreateWindowExW, DefWindowProcW, DestroyWindow,
    DispatchMessageW, ES_AUTOHSCROLL, GWLP_USERDATA, GetMessageW, GetWindowLongPtrW, GetWindowRect,
    GetWindowTextLengthW, GetWindowTextW, HMENU, IDC_ARROW, IsDialogMessageW, LB_ADDSTRING,
    LB_GETCURSEL, LB_SETCURSEL, LBN_DBLCLK, LBS_NOTIFY, LoadCursorW, MSG, PostQuitMessage,
    RegisterClassW, SW_SHOW, SendMessageW, SetWindowLongPtrW, ShowWindow, TranslateMessage,
    WM_COMMAND, WM_CTLCOLORSTATIC, WM_NCCREATE, WM_NCDESTROY, WM_SETFONT, WNDCLASSW, WS_BORDER,
    WS_CAPTION, WS_CHILD, WS_POPUP, WS_SYSMENU, WS_TABSTOP, WS_VISIBLE, WS_VSCROLL,
};

use crate::gdi::Font;

const PROMPT_CLASS: &str = "GrindPromptClass";
static PROMPT_REGISTERED: AtomicBool = AtomicBool::new(false);

const ID_PROMPT_EDIT: usize = 10;

/// What the prompt window owns while it is up.
struct Prompt {
    edit: HWND,
    /// `None` until OK is pressed; the popup is torn down either way.
    answer: Option<String>,
    finished: bool,
    /// Kept alive because `WM_SETFONT` does not copy the handle.
    _font: Option<Font>,
}

/// Ask for one line of text. `None` means the user cancelled or typed nothing.
///
/// A popup of this shell's own rather than a resource-script dialog, because this binary has no
/// resources and `DialogBoxIndirect` would mean building a `DLGTEMPLATE` by hand — more code
/// than a window, and less readable. `IsDialogMessageW` supplies what a dialog would have given:
/// Tab between the controls, Enter for the default button, Escape for Cancel.
///
/// **A nested message loop**, like everything else in this file.
pub fn prompt(owner: HWND, title: &str, label: &str, initial: &str) -> Option<String> {
    let class = gdi::wide(PROMPT_CLASS);
    // SAFETY: the class name outlives every call below, and the boxed state is handed to the
    // popup and taken back in `WM_NCDESTROY`.
    unsafe {
        let instance = GetModuleHandleW(None).ok()?;
        if !PROMPT_REGISTERED.swap(true, Ordering::SeqCst) {
            let wc = WNDCLASSW {
                lpfnWndProc: Some(prompt_proc),
                hInstance: instance.into(),
                lpszClassName: PCWSTR(class.as_ptr()),
                hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
                hbrBackground: HBRUSH(COLOR_BTNFACE.0 as isize as *mut std::ffi::c_void),
                ..Default::default()
            };
            if RegisterClassW(&wc) == 0 {
                PROMPT_REGISTERED.store(false, Ordering::SeqCst);
                return None;
            }
        }

        let dpi = GetDpiForWindow(owner).max(96);
        let px = |value: f64| crate::sheet::geom::scale(value, dpi).round() as i32;
        let (w, h) = (px(360.0), px(150.0));
        let mut owner_rect = Default::default();
        let _ = GetWindowRect(owner, &mut owner_rect);
        let x = owner_rect.left + ((owner_rect.right - owner_rect.left) - w) / 2;
        let y = owner_rect.top + ((owner_rect.bottom - owner_rect.top) - h) / 3;

        let state = Box::new(Prompt {
            edit: HWND::default(),
            answer: None,
            finished: false,
            _font: None,
        });
        let title = gdi::wide(title);
        let Ok(popup) = CreateWindowExW(
            Default::default(),
            PCWSTR(class.as_ptr()),
            PCWSTR(title.as_ptr()),
            WS_POPUP | WS_CAPTION | WS_SYSMENU,
            x,
            y,
            w,
            h,
            Some(owner),
            None::<HMENU>,
            Some(instance.into()),
            Some(Box::into_raw(state).cast()),
        ) else {
            return None;
        };

        let font = Font::new("Segoe UI", px(13.0), false);
        let set_font = |control: HWND| {
            SendMessageW(
                control,
                WM_SETFONT,
                Some(WPARAM(font.handle().0 as usize)),
                Some(LPARAM(1)),
            );
        };
        let child = |class: &str, text: &str, style, id: usize, cx, cy, cw, ch| -> HWND {
            let class = gdi::wide(class);
            let text = gdi::wide(text);
            CreateWindowExW(
                Default::default(),
                PCWSTR(class.as_ptr()),
                PCWSTR(text.as_ptr()),
                style,
                cx,
                cy,
                cw,
                ch,
                Some(popup),
                Some(HMENU(id as *mut std::ffi::c_void)),
                Some(instance.into()),
                None,
            )
            .unwrap_or_default()
        };

        let pad = px(12.0);
        let line = px(22.0);
        let button = (px(84.0), px(26.0));
        let inner = w - pad * 2;
        child(
            "STATIC",
            label,
            WS_CHILD | WS_VISIBLE,
            0,
            pad,
            pad,
            inner,
            line,
        );
        let edit = child(
            "EDIT",
            initial,
            WS_CHILD
                | WS_VISIBLE
                | WS_BORDER
                | WS_TABSTOP
                | windows::Win32::UI::WindowsAndMessaging::WINDOW_STYLE(ES_AUTOHSCROLL as u32),
            ID_PROMPT_EDIT,
            pad,
            pad + line + px(4.0),
            inner,
            px(24.0),
        );
        let row = h - button.1 - px(40.0);
        let ok = child(
            "BUTTON",
            "OK",
            WS_CHILD
                | WS_VISIBLE
                | WS_TABSTOP
                | windows::Win32::UI::WindowsAndMessaging::WINDOW_STYLE(BS_DEFPUSHBUTTON as u32),
            IDOK.0 as usize,
            w - pad - button.0 * 2 - px(8.0),
            row,
            button.0,
            button.1,
        );
        let cancel = child(
            "BUTTON",
            "Cancel",
            WS_CHILD | WS_VISIBLE | WS_TABSTOP,
            IDCANCEL.0 as usize,
            w - pad - button.0,
            row,
            button.0,
            button.1,
        );
        for control in [edit, ok, cancel] {
            set_font(control);
        }
        with_prompt(popup, |prompt| {
            prompt.edit = edit;
            prompt._font = Some(font);
        });

        // Modal: the owner is disabled for exactly as long as the popup is up, and re-enabled
        // *before* it is destroyed, or Windows activates some other application instead.
        let _ = EnableWindow(owner, false);
        let _ = ShowWindow(popup, SW_SHOW);
        let _ = SetFocus(Some(edit));
        SendMessageW(edit, EM_SETSEL, Some(WPARAM(0)), Some(LPARAM(-1)));

        let mut message = MSG::default();
        loop {
            let finished = with_prompt(popup, |prompt| prompt.finished).unwrap_or(true);
            if finished {
                break;
            }
            let got = GetMessageW(&mut message, None, 0, 0).0;
            if got <= 0 {
                // `WM_QUIT` arrived while a modal was up — the application is closing. Put it
                // back so the outer loop sees it too, and stop.
                PostQuitMessage(0);
                break;
            }
            if IsDialogMessageW(popup, &message).as_bool() {
                continue;
            }
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }

        let answer = with_prompt(popup, |prompt| prompt.answer.take()).flatten();
        let _ = EnableWindow(owner, true);
        let _ = SetActiveWindow(owner);
        let _ = DestroyWindow(popup);
        answer.filter(|text| !text.trim().is_empty())
    }
}

/// Run `f` with the popup's state. The same arrangement `win.rs` uses, and the same rule: the
/// borrow lives for the call and nothing inside it dispatches a message.
unsafe fn with_prompt<T>(hwnd: HWND, f: impl FnOnce(&mut Prompt) -> T) -> Option<T> {
    // SAFETY: the slot holds either null or the pointer stored in `WM_NCCREATE`.
    let raw = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut Prompt;
    if raw.is_null() {
        return None;
    }
    // SAFETY: exclusive for the duration of this call.
    Some(f(unsafe { &mut *raw }))
}

extern "system" fn prompt_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_NCCREATE => {
            // SAFETY: `lparam` is this message's `CREATESTRUCTW`.
            unsafe {
                let create = &*(lparam.0 as *const CREATESTRUCTW);
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
                DefWindowProcW(hwnd, message, wparam, lparam)
            }
        }
        // The label sits on the dialog's own face rather than on a white patch of its own.
        WM_CTLCOLORSTATIC => {
            // SAFETY: `wparam` is the control's `HDC` for this message.
            unsafe {
                SetBkMode(HDC(wparam.0 as *mut std::ffi::c_void), TRANSPARENT);
                SetTextColor(
                    HDC(wparam.0 as *mut std::ffi::c_void),
                    COLORREF(GetSysColor(COLOR_BTNTEXT)),
                );
                LRESULT(GetSysColorBrush(COLOR_BTNFACE).0 as isize)
            }
        }
        WM_COMMAND => {
            let id = (wparam.0 & 0xffff) as i32;
            let commit = id == IDOK.0;
            if commit || id == IDCANCEL.0 {
                // SAFETY: one borrow, and reading the control's text does not dispatch.
                unsafe {
                    with_prompt(hwnd, |prompt| {
                        if commit {
                            prompt.answer = Some(window_text(prompt.edit));
                        }
                        prompt.finished = true;
                    });
                }
            }
            LRESULT(0)
        }
        WM_NCDESTROY => {
            // SAFETY: the pointer came from `Box::into_raw`; reconstituting it once frees it.
            unsafe {
                let raw = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Prompt;
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
// The chooser — the text pane's outline dialog
// ---------------------------------------------------------------------------

const CHOOSER_CLASS: &str = "GrindChooserClass";
static CHOOSER_REGISTERED: AtomicBool = AtomicBool::new(false);

const ID_CHOOSER_LIST: usize = 10;

/// What the popup owns while it is up. The same shape as [`Prompt`] with a list in place of an
/// edit box, and the same reason it is a separate type: `GWLP_USERDATA` holds one pointer, so a
/// dialog with a list and one with a line of text cannot share a state struct without one of
/// them carrying a field that means nothing to it.
struct Chooser {
    list: HWND,
    /// The chosen row, or `None` until OK or a double-click accepts one.
    answer: Option<usize>,
    finished: bool,
    _font: Option<Font>,
}

/// Ask the user to pick one of a list of lines. `None` means cancelled, or nothing to pick from.
///
/// This is `grind text`'s outline dialog (`App::outline`, `doc/windows-shell.md`'s W5b) and W6's
/// "Show Source" and "Check Document", built generically over strings rather than over `Heading`
/// or a `Diagnostic` so this module stays ignorant of the document types (R8's rule for the core
/// applies just as well to a shell file with no reason to know one). A double-click accepts the
/// same as OK, because a list a user has to click twice — once to select, once on a separate
/// button — is slower than the box it replaces.
///
/// `initial` is the row already selected when the list opens — "the line the selection is on
/// marked" (D9's own wording), clamped rather than refused so a caller need not check its own
/// bound first. A plain `LB_SETCURSEL` is enough to make it visible too: Windows scrolls a
/// listbox to the row a program selects, the same as it does for one a person clicks.
pub fn choose(owner: HWND, title: &str, items: &[String], initial: usize) -> Option<usize> {
    if items.is_empty() {
        return None;
    }
    let class = gdi::wide(CHOOSER_CLASS);
    // SAFETY: the class name outlives every call below, and the boxed state is handed to the
    // popup and taken back in `WM_NCDESTROY`.
    unsafe {
        let instance = GetModuleHandleW(None).ok()?;
        if !CHOOSER_REGISTERED.swap(true, Ordering::SeqCst) {
            let wc = WNDCLASSW {
                lpfnWndProc: Some(chooser_proc),
                hInstance: instance.into(),
                lpszClassName: PCWSTR(class.as_ptr()),
                hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
                hbrBackground: HBRUSH(COLOR_BTNFACE.0 as isize as *mut std::ffi::c_void),
                ..Default::default()
            };
            if RegisterClassW(&wc) == 0 {
                CHOOSER_REGISTERED.store(false, Ordering::SeqCst);
                return None;
            }
        }

        let dpi = GetDpiForWindow(owner).max(96);
        let px = |value: f64| crate::sheet::geom::scale(value, dpi).round() as i32;
        // Wider than it was, because of what goes in it: a `LISTBOX` clips rather than scrolls
        // sideways, and W9's function list is four columns — name, plain-English alias, category
        // and the spec's own summary — where the outline and the lint pane were one short line
        // each. The source view wanted it too; a projection line is as long as the cell it
        // projects.
        let (w, h) = (px(620.0), px(420.0));
        let mut owner_rect = Default::default();
        let _ = GetWindowRect(owner, &mut owner_rect);
        let x = owner_rect.left + ((owner_rect.right - owner_rect.left) - w) / 2;
        let y = owner_rect.top + ((owner_rect.bottom - owner_rect.top) - h) / 3;

        let state = Box::new(Chooser {
            list: HWND::default(),
            answer: None,
            finished: false,
            _font: None,
        });
        let title = gdi::wide(title);
        let Ok(popup) = CreateWindowExW(
            Default::default(),
            PCWSTR(class.as_ptr()),
            PCWSTR(title.as_ptr()),
            WS_POPUP | WS_CAPTION | WS_SYSMENU,
            x,
            y,
            w,
            h,
            Some(owner),
            None::<HMENU>,
            Some(instance.into()),
            Some(Box::into_raw(state).cast()),
        ) else {
            return None;
        };

        let font = Font::new("Segoe UI", px(13.0), false);
        let set_font = |control: HWND| {
            SendMessageW(
                control,
                WM_SETFONT,
                Some(WPARAM(font.handle().0 as usize)),
                Some(LPARAM(1)),
            );
        };
        let child = |class: &str, text: &str, style, id: usize, cx, cy, cw, ch| -> HWND {
            let class = gdi::wide(class);
            let text = gdi::wide(text);
            CreateWindowExW(
                Default::default(),
                PCWSTR(class.as_ptr()),
                PCWSTR(text.as_ptr()),
                style,
                cx,
                cy,
                cw,
                ch,
                Some(popup),
                Some(HMENU(id as *mut std::ffi::c_void)),
                Some(instance.into()),
                None,
            )
            .unwrap_or_default()
        };

        let pad = px(12.0);
        let button = (px(84.0), px(26.0));
        let inner = w - pad * 2;
        let row = h - button.1 - px(40.0);
        let list = child(
            "LISTBOX",
            "",
            WS_CHILD
                | WS_VISIBLE
                | WS_BORDER
                | WS_TABSTOP
                | WS_VSCROLL
                | windows::Win32::UI::WindowsAndMessaging::WINDOW_STYLE(LBS_NOTIFY as u32),
            ID_CHOOSER_LIST,
            pad,
            pad,
            inner,
            row - pad * 2,
        );
        for item in items {
            let item = gdi::wide(item);
            SendMessageW(
                list,
                LB_ADDSTRING,
                Some(WPARAM(0)),
                Some(LPARAM(item.as_ptr() as isize)),
            );
        }
        let initial = initial.min(items.len() - 1);
        SendMessageW(list, LB_SETCURSEL, Some(WPARAM(initial)), Some(LPARAM(0)));
        let ok = child(
            "BUTTON",
            "OK",
            WS_CHILD
                | WS_VISIBLE
                | WS_TABSTOP
                | windows::Win32::UI::WindowsAndMessaging::WINDOW_STYLE(BS_DEFPUSHBUTTON as u32),
            IDOK.0 as usize,
            w - pad - button.0 * 2 - px(8.0),
            row,
            button.0,
            button.1,
        );
        let cancel = child(
            "BUTTON",
            "Cancel",
            WS_CHILD | WS_VISIBLE | WS_TABSTOP,
            IDCANCEL.0 as usize,
            w - pad - button.0,
            row,
            button.0,
            button.1,
        );
        for control in [list, ok, cancel] {
            set_font(control);
        }
        with_chooser(popup, |chooser| {
            chooser.list = list;
            chooser._font = Some(font);
        });

        // Modal, the same pairing `prompt` uses: the owner is disabled for exactly as long as
        // the popup is up, and re-enabled before it is destroyed.
        let _ = EnableWindow(owner, false);
        let _ = ShowWindow(popup, SW_SHOW);
        let _ = SetFocus(Some(list));

        let mut message = MSG::default();
        loop {
            let finished = with_chooser(popup, |chooser| chooser.finished).unwrap_or(true);
            if finished {
                break;
            }
            let got = GetMessageW(&mut message, None, 0, 0).0;
            if got <= 0 {
                PostQuitMessage(0);
                break;
            }
            if IsDialogMessageW(popup, &message).as_bool() {
                continue;
            }
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }

        let answer = with_chooser(popup, |chooser| chooser.answer.take()).flatten();
        let _ = EnableWindow(owner, true);
        let _ = SetActiveWindow(owner);
        let _ = DestroyWindow(popup);
        answer
    }
}

/// Run `f` with the popup's state — [`with_prompt`]'s twin, over [`Chooser`].
unsafe fn with_chooser<T>(hwnd: HWND, f: impl FnOnce(&mut Chooser) -> T) -> Option<T> {
    // SAFETY: the slot holds either null or the pointer stored in `WM_NCCREATE`.
    let raw = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut Chooser;
    if raw.is_null() {
        return None;
    }
    // SAFETY: exclusive for the duration of this call.
    Some(f(unsafe { &mut *raw }))
}

extern "system" fn chooser_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_NCCREATE => {
            // SAFETY: `lparam` is this message's `CREATESTRUCTW`.
            unsafe {
                let create = &*(lparam.0 as *const CREATESTRUCTW);
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
                DefWindowProcW(hwnd, message, wparam, lparam)
            }
        }
        WM_CTLCOLORSTATIC => {
            // SAFETY: `wparam` is the control's `HDC` for this message.
            unsafe {
                SetBkMode(HDC(wparam.0 as *mut std::ffi::c_void), TRANSPARENT);
                SetTextColor(
                    HDC(wparam.0 as *mut std::ffi::c_void),
                    COLORREF(GetSysColor(COLOR_BTNTEXT)),
                );
                LRESULT(GetSysColorBrush(COLOR_BTNFACE).0 as isize)
            }
        }
        WM_COMMAND => {
            let id = (wparam.0 & 0xffff) as i32;
            let notification = ((wparam.0 >> 16) & 0xffff) as u32;
            let commit =
                id == IDOK.0 || (id as usize == ID_CHOOSER_LIST && notification == LBN_DBLCLK);
            if commit || id == IDCANCEL.0 {
                // SAFETY: one borrow, and reading the selection does not dispatch.
                unsafe {
                    with_chooser(hwnd, |chooser| {
                        if commit {
                            let selection =
                                SendMessageW(chooser.list, LB_GETCURSEL, Some(WPARAM(0)), None).0;
                            if selection >= 0 {
                                chooser.answer = Some(selection as usize);
                            }
                        }
                        chooser.finished = true;
                    });
                }
            }
            LRESULT(0)
        }
        WM_NCDESTROY => {
            // SAFETY: the pointer came from `Box::into_raw`; reconstituting it once frees it.
            unsafe {
                let raw = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Chooser;
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
