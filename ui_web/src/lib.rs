// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `grind-web` — the browser shell, over **both** document types.
//!
//! A third kind of shell. `grind-cli` and `grind-tui` are Rust calling the core directly and
//! the GTK shells are Rust through GTK's bindings; this one is Rust compiled to
//! `wasm32-unknown-unknown`, talking to the page through `wasm-bindgen`. The cores are
//! ordinary Cargo dependencies — there is no FFI layer, because the shell is Rust too.
//!
//! **This is the honest test of rule 5** (doc/plan.md: *do not assume a filesystem*). The
//! browser has no paths. A document arrives from the File API as bytes and leaves as a
//! download, which is exactly what `App::open_bytes` and `App::save_bytes` exist for — the
//! shell never learns a path, because there isn't one. Neither core needed a change to run
//! here, which is the whole point of having paired every `*_file` with a `*_bytes`.
//!
//! **One bundle, two document types** (`doc/suite.md`, R10 and S10). Unlike GTK, the web
//! shell does not split per type: a document arrives from a file picker as bytes with no
//! path and no mime association, so there is nothing for a second bundle to be associated
//! *with*. `grind_core::kind` reads the bytes, the shell shows that pane, and the toolbar
//! above them is the same toolbar either way.
//!
//! This file is that shell: the chrome both panes share (open, save, undo, redo, the
//! document's name), the dispatch, and the frame scheduling. Everything about a *grid* is in
//! [`sheet`] and everything about a *document* is in [`text`].

pub mod sheet;
pub mod text;

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use grind_core::{DocumentKind, Form, Observer, kind};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::{JsFuture, spawn_local};
use web_sys::{
    Document, Event, File, HtmlAnchorElement, HtmlButtonElement, HtmlElement, HtmlInputElement,
};

/// What a document nobody has opened is downloaded as — one per document type. The package
/// forms rather than the flat ones, because they are what every other application opens
/// without being told.
const UNTITLED_SHEET: &str = "untitled.ods";
const UNTITLED_TEXT: &str = "untitled.odt";

thread_local! {
    /// The live shell, so an animation-frame callback can find its way back. The page owns
    /// it until the tab closes; nothing ever takes it out again.
    static SHELL: RefCell<Option<Rc<Shell>>> = const { RefCell::new(None) };
}

/// Which document type is open. There is always exactly one — an empty spreadsheet at
/// startup, because a page has to show something and a grid is what this shell was first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Sheet,
    Text,
}

/// Called by the generated glue as soon as the module is instantiated.
#[wasm_bindgen(start)]
pub fn start() -> Result<(), JsValue> {
    let window = web_sys::window().ok_or_else(|| JsValue::from_str("no window"))?;
    let document = window
        .document()
        .ok_or_else(|| JsValue::from_str("no document"))?;

    // One flag for both panes: it says "a repaint is owed", and which pane owes it is
    // whichever one is showing.
    let pending = Arc::new(AtomicBool::new(false));
    let sheet_app = Arc::new(grind_sheet::App::new());
    let text_app = Arc::new(grind_text::App::new());

    // The core pushes, the shell never polls (doc/plan.md rule 3) — the same contract
    // `grind-tui` implements with a flag and the GTK shells with a channel.
    sheet_app.set_observer(Arc::new(Notifier(pending.clone())));
    text_app.set_observer(Arc::new(Notifier(pending.clone())));

    let shell = Rc::new(Shell {
        sheet: sheet::Ui::new(&document, sheet_app, pending.clone())?,
        text: text::Ui::new(&document, text_app, pending.clone())?,
        dom: Chrome::find(&document)?,
        pending,
        mode: Cell::new(Mode::Sheet),
        name: RefCell::new(String::new()),
    });

    // The page's own two "declared in Rust, used in CSS" numbers — see each function.
    text::declare_spacing(&document)?;

    wire_toolbar(&shell)?;
    wire_file_input(&shell)?;
    wire_window(&shell, &window)?;

    SHELL.with(|slot| *slot.borrow_mut() = Some(shell.clone()));

    shell.show(Mode::Sheet)?;

    // `?doc=<url>` opens that document at startup — the page's own address is the only way
    // to point this shell at a file without a picker.
    if let Ok(search) = window.location().search()
        && let Some(url) = doc_param(&search)
    {
        spawn_local(shell.clone().fetch(url));
    }

    shell.refresh();
    Ok(())
}

/// The `doc` parameter of a query string, still percent-encoded — which is the form `fetch`
/// wants, so nothing here decodes it.
fn doc_param(search: &str) -> Option<String> {
    search
        .trim_start_matches('?')
        .split('&')
        .find_map(|pair| pair.strip_prefix("doc="))
        .filter(|url| !url.is_empty())
        .map(str::to_owned)
}

/// The chrome above the panes: the same buttons whichever document is open.
struct Chrome {
    document: Document,
    sheet_pane: HtmlElement,
    text_pane: HtmlElement,
    formula_bar: HtmlElement,
    /// The sheet tabs. A document has no sheets, so in text mode they go with the grid.
    tabs: HtmlElement,
    name: HtmlElement,
    open: HtmlButtonElement,
    save: HtmlButtonElement,
    undo: HtmlButtonElement,
    redo: HtmlButtonElement,
    recalc: HtmlButtonElement,
    file_input: HtmlInputElement,
}

impl Chrome {
    fn find(document: &Document) -> Result<Self, JsValue> {
        Ok(Chrome {
            document: document.clone(),
            sheet_pane: element(document, "surface")?,
            text_pane: element(document, "page")?,
            formula_bar: element(document, "formula-bar")?,
            tabs: element(document, "tabs")?,
            name: element(document, "name")?,
            open: element(document, "open")?,
            save: element(document, "save")?,
            undo: element(document, "undo")?,
            redo: element(document, "redo")?,
            recalc: element(document, "recalc")?,
            file_input: element(document, "file-input")?,
        })
    }
}

/// The whole shell: two panes, one chrome, one document open at a time.
struct Shell {
    sheet: Rc<sheet::Ui>,
    text: Rc<text::Ui>,
    dom: Chrome,
    /// Set by either observer, cleared by the repaint it asked for.
    pending: Arc<AtomicBool>,
    mode: Cell<Mode>,
    /// The name the document arrived under, so a download has something to be called. Not a
    /// path — there are none here.
    name: RefCell<String>,
}

impl Shell {
    /// Repaint the pane that is showing, and the chrome above it.
    fn refresh(&self) {
        self.pending.store(false, Ordering::SeqCst);
        match self.mode.get() {
            Mode::Sheet => self.sheet.refresh(),
            Mode::Text => self.text.refresh(),
        }
        if let Err(error) = self.refresh_chrome() {
            web_sys::console::error_1(&error);
        }
    }

    fn refresh_chrome(&self) -> Result<(), JsValue> {
        let name = self.document_name();
        self.dom.name.set_text_content(Some(&name));
        self.dom.undo.set_disabled(!self.can_undo());
        self.dom.redo.set_disabled(!self.can_redo());
        let what = match self.mode.get() {
            Mode::Sheet => "sheet",
            Mode::Text => "text",
        };
        self.dom.document.set_title(&format!("{name} — {what}"));
        Ok(())
    }

    /// Put one pane on screen and take the other one off.
    ///
    /// `hidden` rather than a class, because a hidden element must not be *focusable* — a
    /// pane that is only invisible still takes the keyboard, and then every keystroke goes
    /// to the document nobody can see.
    fn show(&self, mode: Mode) -> Result<(), JsValue> {
        self.mode.set(mode);
        let sheet = mode == Mode::Sheet;
        self.dom.sheet_pane.set_hidden(!sheet);
        self.dom.formula_bar.set_hidden(!sheet);
        self.dom.tabs.set_hidden(!sheet);
        self.dom.text_pane.set_hidden(sheet);
        // Recalculation is a spreadsheet's word. The button goes rather than greying out:
        // there is no such thing as an unrecalculated paragraph.
        self.dom.recalc.set_hidden(!sheet);
        match sheet {
            true => self.sheet.focus(),
            false => self.text.focus(),
        }
    }

    fn can_undo(&self) -> bool {
        match self.mode.get() {
            Mode::Sheet => self.sheet.app.can_undo(),
            Mode::Text => self.text.app.can_undo(),
        }
    }

    fn can_redo(&self) -> bool {
        match self.mode.get() {
            Mode::Sheet => self.sheet.app.can_redo(),
            Mode::Text => self.text.app.can_redo(),
        }
    }

    fn undo(&self) {
        let moved = match self.mode.get() {
            Mode::Sheet => self.sheet.app.undo(),
            Mode::Text => self.text.app.undo(),
        };
        if !moved {
            self.set_message("Nothing to undo".to_owned());
        }
    }

    fn redo(&self) {
        let moved = match self.mode.get() {
            Mode::Sheet => self.sheet.app.redo(),
            Mode::Text => self.text.app.redo(),
        };
        if !moved {
            self.set_message("Nothing to redo".to_owned());
        }
    }

    fn set_message(&self, message: String) {
        match self.mode.get() {
            Mode::Sheet => self.sheet.set_message(message),
            Mode::Text => self.text.set_message(message),
        }
    }

    // --- documents ---

    fn open_picker(&self) {
        // Cleared first, or picking the same file twice fires no change event and the second
        // open silently does nothing.
        self.dom.file_input.set_value("");
        self.dom.file_input.click();
    }

    /// Read a picked file into the core.
    ///
    /// **Bytes, not text** — an `.ods` is a zip, and the name is all the browser gives us. It
    /// travels with the document only so a download has something to be called; there is no
    /// path to write back to.
    async fn load(self: Rc<Self>, file: File) {
        let name = file.name();
        let buffer = match JsFuture::from(file.array_buffer()).await {
            Ok(buffer) => buffer,
            Err(_) => return self.set_message(format!("Could not read {name}")),
        };
        self.open(name, &js_sys::Uint8Array::new(&buffer).to_vec());
    }

    /// A document named in the page's own URL — `?doc=sample.fodt` — fetched and opened as if
    /// it had been picked.
    ///
    /// The browser hands a page no path and no way to preload the file picker, so a document
    /// *served next to the page* has no other way in. That is the whole point: `scripts/run.sh
    /// web` puts a sample document in `dist/` and prints the URL, which is how this shell gets
    /// demo data in front of it without a picker and without a second open path — the bytes
    /// end up in `App::open_bytes` either way.
    async fn fetch(self: Rc<Self>, url: String) {
        let Some(window) = web_sys::window() else {
            return;
        };
        let failed = || format!("Could not load {url}");
        let response = match JsFuture::from(window.fetch_with_str(&url)).await {
            Ok(response) => web_sys::Response::from(response),
            Err(_) => return self.set_message(failed()),
        };
        if !response.ok() {
            return self.set_message(format!("{url}: {} {}", response.status(), failed()));
        }
        let buffer = match response.array_buffer() {
            Ok(promise) => JsFuture::from(promise).await,
            Err(error) => Err(error),
        };
        let Ok(buffer) = buffer else {
            return self.set_message(failed());
        };
        // The last path segment is the document's name — all a download needs, and the only
        // thing a name is used for here.
        let name = url.rsplit('/').next().unwrap_or(&url).to_owned();
        self.open(name, &js_sys::Uint8Array::new(&buffer).to_vec());
    }

    /// Bytes in, and the pane that can show them.
    ///
    /// **The kind decides, not the extension** (`grind_core::kind`): both readers are
    /// tolerant by construction, so handing a spreadsheet to the text reader would produce an
    /// empty document rather than an error. Deciding first is what makes one bundle able to
    /// hold two document types honestly.
    fn open(&self, name: String, bytes: &[u8]) {
        let opened = match kind(bytes) {
            Some(DocumentKind::Spreadsheet) => self.sheet.open(&name, bytes).map(|()| Mode::Sheet),
            Some(DocumentKind::Text) => self.text.open(&name, bytes).map(|()| Mode::Text),
            // A presentation is a document type this suite reserves and does not have
            // (`doc/suite.md`); anything else is not an ODF document at all.
            other => Err(match other {
                Some(kind) => {
                    format!("{name} is a {kind:?} document — this build has no editor for one")
                }
                None => format!("{name} is not an OpenDocument file"),
            }),
        };
        match opened {
            Ok(mode) => {
                *self.name.borrow_mut() = name.clone();
                if let Err(error) = self.show(mode) {
                    web_sys::console::error_1(&error);
                }
                self.set_message(format!("Opened {name}"));
            }
            // Tolerance is the reader's job, not the shell's: if the core will not take it,
            // saying why is all there is to do.
            Err(error) => self.set_message(error),
        }
    }

    /// Save by handing the document to a download — the only writing a page may do.
    ///
    /// The form follows the name the document arrived under, so an `.fodt` opened here goes
    /// back as flat XML and an `.odt` as a package. R6's spliced writer is underneath: a
    /// document that was only edited comes back as the file it was, with those paragraphs
    /// replaced.
    fn save(&self) {
        let name = self.document_name();
        let form = form_of(&name);
        let bytes = match self.mode.get() {
            Mode::Sheet => self.sheet.save_bytes(form),
            Mode::Text => self.text.save_bytes(form),
        };
        let bytes = match bytes {
            Ok(bytes) => bytes,
            Err(error) => return self.set_message(error),
        };
        match self.download(&name, &bytes) {
            Ok(()) => self.set_message(format!("Saved {name} to your downloads")),
            Err(_) => self.set_message(format!("The browser refused to download {name}")),
        }
    }

    fn download(&self, name: &str, bytes: &[u8]) -> Result<(), JsValue> {
        let parts = js_sys::Array::new();
        parts.push(&js_sys::Uint8Array::from(bytes));
        let blob = web_sys::Blob::new_with_u8_array_sequence(&parts)?;
        let url = web_sys::Url::create_object_url_with_blob(&blob)?;

        let anchor: HtmlAnchorElement = self
            .dom
            .document
            .create_element("a")?
            .dyn_into()
            .map_err(|_| JsValue::from_str("an anchor is not an anchor"))?;
        anchor.set_href(&url);
        anchor.set_download(name);
        anchor.click();

        // The blob would otherwise be held until the tab closes.
        web_sys::Url::revoke_object_url(&url)
    }

    fn document_name(&self) -> String {
        let name = self.name.borrow();
        if !name.is_empty() {
            return name.clone();
        }
        match self.mode.get() {
            Mode::Sheet => UNTITLED_SHEET.to_owned(),
            Mode::Text => UNTITLED_TEXT.to_owned(),
        }
    }
}

/// Reach the live shell from a pane — the chrome's actions belong to it, and a pane that
/// held an `Rc` to its owner would be a cycle that never drops.
pub(crate) fn with_shell(f: impl FnOnce(&Shell)) {
    let shell = SHELL.with(|slot| slot.borrow().clone());
    if let Some(shell) = shell {
        f(&shell);
    }
}

/// Ask for a repaint the core will not send: a scroll, a selection, a resize — anything that
/// changes the picture without changing the document.
pub(crate) fn request_repaint(pending: &AtomicBool) {
    if !pending.swap(true, Ordering::SeqCst) {
        request_frame();
    }
}

/// Raises a repaint when a core changes, and schedules the frame that draws it.
///
/// [`Observer`] is `Send + Sync`, so this may not hold anything from the page — a wasm module
/// is single-threaded but the trait does not know that. It holds a flag instead, which
/// doubles as "a frame is already scheduled": an edit that notifies twice still repaints once.
struct Notifier(Arc<AtomicBool>);

impl Observer for Notifier {
    fn changed(&self) {
        request_repaint(&self.0);
    }
}

pub(crate) fn request_frame() {
    let Some(window) = web_sys::window() else {
        return;
    };
    // `once_into_js` hands the closure to JS and frees it after the call, which is what makes
    // a per-frame allocation acceptable.
    let callback = Closure::once_into_js(move || with_shell(|shell| shell.refresh()));
    let _ = window.request_animation_frame(callback.unchecked_ref());
}

pub(crate) fn element<T: JsCast>(document: &Document, id: &str) -> Result<T, JsValue> {
    document
        .get_element_by_id(id)
        .ok_or_else(|| JsValue::from_str(&format!("index.html is missing #{id}")))?
        .dyn_into::<T>()
        .map_err(|_| JsValue::from_str(&format!("#{id} is not the element this shell expects")))
}

/// Which ODF form a name asks for. Anything that is not flat XML is a package, which is also
/// the right answer for a name with no extension at all.
pub(crate) fn form_of(name: &str) -> Form {
    match name
        .rsplit('.')
        .next()
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("fods") | Some("fodt") | Some("xml") => Form::Flat,
        _ => Form::Package,
    }
}

pub(crate) fn js(error: impl std::fmt::Display) -> JsValue {
    JsValue::from_str(&error.to_string())
}

// --- wiring ---

fn wire_toolbar(shell: &Rc<Shell>) -> Result<(), JsValue> {
    // Every button hands the keyboard back: a toolbar that keeps focus leaves the user
    // typing into nothing.
    type Press = fn(&Shell);
    // The flag is "does this button take the focus somewhere on purpose" — opening puts it
    // in the file picker, which is the one place it should stay.
    for (button, press, keeps_focus) in [
        (
            &shell.dom.open,
            (|s: &Shell| s.open_picker()) as Press,
            true,
        ),
        (&shell.dom.save, |s| s.save(), false),
        (&shell.dom.undo, |s| s.undo(), false),
        (&shell.dom.redo, |s| s.redo(), false),
        (
            &shell.dom.recalc,
            |s| {
                let _ = s.sheet.apply(sheet::keymap::Action::Recalc);
            },
            false,
        ),
    ] {
        let shell = shell.clone();
        listen(button, "click", move |_: Event| {
            press(&shell);
            if !keeps_focus {
                let _ = shell.show(shell.mode.get());
            }
        })?;
    }
    Ok(())
}

fn wire_file_input(shell: &Rc<Shell>) -> Result<(), JsValue> {
    let input = shell.dom.file_input.clone();
    let shell = shell.clone();
    listen(&input, "change", move |_: Event| {
        let Some(file) = shell.dom.file_input.files().and_then(|files| files.get(0)) else {
            return;
        };
        // Reading a file is a promise; nothing else in this shell is async.
        spawn_local(shell.clone().load(file));
    })
}

fn wire_window(shell: &Rc<Shell>, window: &web_sys::Window) -> Result<(), JsValue> {
    // A resize changes how much fits and, for a document, where every line breaks — which
    // only a repaint can discover.
    let resize = shell.clone();
    listen(window, "resize", move |_: Event| {
        request_repaint(&resize.pending);
    })?;

    // The browser's answer to "save before closing?". A page may ask for the prompt but not
    // word it, so there is nothing to phrase here.
    let unload = shell.clone();
    listen(window, "beforeunload", move |event: Event| {
        if !unload.can_undo() {
            return;
        }
        event.prevent_default();
        // Chrome still wants the legacy property set as well.
        let _ = js_sys::Reflect::set(
            &event,
            &JsValue::from_str("returnValue"),
            &JsValue::from_str(""),
        );
    })
}

/// Attach a listener for the lifetime of the page.
///
/// The closure is deliberately leaked: it lives exactly as long as the element it is attached
/// to, and both die with the tab.
pub(crate) fn listen<E, F>(
    target: &web_sys::EventTarget,
    event: &str,
    handler: F,
) -> Result<(), JsValue>
where
    // Any DOM event type: `FromWasmAbi` is what lets JS hand it to a Rust closure.
    E: wasm_bindgen::convert::FromWasmAbi + 'static,
    F: FnMut(E) + 'static,
{
    let closure = Closure::wrap(Box::new(handler) as Box<dyn FnMut(E)>);
    target.add_event_listener_with_callback(event, closure.as_ref().unchecked_ref())?;
    closure.forget();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_page_can_be_told_which_document_to_open() {
        assert_eq!(
            doc_param("?doc=sample.fods").as_deref(),
            Some("sample.fods")
        );
        assert_eq!(
            doc_param("?x=1&doc=a%20b.ods").as_deref(),
            Some("a%20b.ods")
        );
        assert_eq!(doc_param(""), None);
        assert_eq!(doc_param("?doc="), None);
    }

    /// One rule for both document types: flat XML is named by an `f`, everything else is a
    /// package.
    #[test]
    fn the_download_form_follows_the_name() {
        assert_eq!(form_of("book.fods"), Form::Flat);
        assert_eq!(form_of("BOOK.FODS"), Form::Flat);
        assert_eq!(form_of("report.fodt"), Form::Flat);
        assert_eq!(form_of("book.ods"), Form::Package);
        assert_eq!(form_of("report.odt"), Form::Package);
        assert_eq!(form_of("book"), Form::Package);
    }
}
