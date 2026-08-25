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
//! command palette, the colour grid, the clipboard, the document's name), the dispatch, and
//! the frame scheduling. Everything about a *grid* is in [`sheet`] and everything about a
//! *document* is in [`text`].
//!
//! **It is a web page, not a window.** There is no menu bar to imitate and no window manager
//! to borrow chrome from, so this shell does not pretend: one bar of verbs, one row of tools
//! for whichever document is open, and [`command`]'s palette — Ctrl+K — for everything else,
//! including going somewhere. A file dropped on the page opens; the clipboard is the
//! browser's own, through the events every browser already sends.

pub mod command;
pub mod palette;
pub mod sheet;
pub mod swatch;
pub mod text;

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use grind_core::{DocumentKind, Form, Observer, kind};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::{JsFuture, spawn_local};
use web_sys::{
    ClipboardEvent, Document, DragEvent, Element, Event, File, HtmlAnchorElement,
    HtmlButtonElement, HtmlElement, HtmlInputElement, KeyboardEvent, MouseEvent,
};

use command::Entry;

/// What a document nobody has opened is downloaded as — one per document type.
///
/// The **flat** forms, per `doc/flat-first.md`: a download nobody named is exactly the case
/// that decision covers, and a browser download is the likeliest thing to land straight in a
/// repository. Every other application opens flat ODF too; it is only less familiar, which is
/// a reason to lead with it rather than a reason to hide it.
const UNTITLED_SHEET: &str = "untitled.fods";
const UNTITLED_TEXT: &str = "untitled.fodt";

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
        palette: palette::Palette::find(&document)?,
        swatches: Rc::new(swatch::Swatches::find(&document)?),
        pending,
        mode: Cell::new(Mode::Sheet),
        name: RefCell::new(String::new()),
    });

    // The page's own two "declared in Rust, used in CSS" numbers — see each function.
    text::declare_spacing(&document)?;

    wire_toolbar(&shell)?;
    wire_file_input(&shell)?;
    wire_palette(&shell)?;
    wire_swatches(&shell)?;
    wire_clipboard(&shell, &document)?;
    wire_dropping(&shell, &document)?;
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
    /// The two tool rows — one per document type, and only one of them ever on screen.
    sheet_tools: HtmlElement,
    text_tools: HtmlElement,
    /// The sheet tabs and their `+`. A document has no sheets, so both go with the grid.
    tabs: HtmlElement,
    sheet_add: HtmlButtonElement,
    name: HtmlElement,
    undo: HtmlButtonElement,
    redo: HtmlButtonElement,
    recalc: HtmlButtonElement,
    palette_open: HtmlButtonElement,
    file_input: HtmlInputElement,
    /// The overlay shown while a file is being dragged across the page.
    drop: HtmlElement,
}

impl Chrome {
    fn find(document: &Document) -> Result<Self, JsValue> {
        Ok(Chrome {
            document: document.clone(),
            sheet_pane: element(document, "surface")?,
            text_pane: element(document, "page")?,
            formula_bar: element(document, "formula-bar")?,
            sheet_tools: element(document, "sheet-tools")?,
            text_tools: element(document, "text-tools")?,
            tabs: element(document, "tabs")?,
            sheet_add: element(document, "sheet-add")?,
            name: element(document, "name")?,
            undo: element(document, "undo")?,
            redo: element(document, "redo")?,
            recalc: element(document, "recalc")?,
            palette_open: element(document, "palette-open")?,
            file_input: element(document, "file-input")?,
            drop: element(document, "drop")?,
        })
    }
}

/// The whole shell: two panes, one chrome, one document open at a time.
struct Shell {
    sheet: Rc<sheet::Ui>,
    text: Rc<text::Ui>,
    dom: Chrome,
    palette: palette::Palette,
    swatches: Rc<swatch::Swatches>,
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
        // The tool row shows what the *selection* already is, so pressing Bold on bold text
        // reads as "this is bold" rather than as "make it bold again".
        match self.mode.get() {
            Mode::Sheet => self.sheet.refresh_tools(),
            Mode::Text => self.text.refresh_tools(),
        }
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
        self.dom.sheet_add.set_hidden(!sheet);
        self.dom.sheet_tools.set_hidden(!sheet);
        self.dom.text_tools.set_hidden(sheet);
        self.dom.text_pane.set_hidden(sheet);
        // Recalculation is a spreadsheet's word. The button goes rather than greying out:
        // there is no such thing as an unrecalculated paragraph.
        self.dom.recalc.set_hidden(!sheet);
        match sheet {
            true => self.sheet.focus(),
            false => self.text.focus(),
        }
    }

    // --- the command palette ---

    /// What the palette shows for a query: whatever the pane can *go to*, then the verbs.
    ///
    /// Targets first because they are the specific answer — someone who typed `B12` meant the
    /// cell, and someone who typed `bold` gets no targets at all.
    fn entries(&self, query: &str) -> Vec<Entry> {
        let (table, mut entries) = match self.mode.get() {
            Mode::Sheet => (command::SHEET, self.sheet.targets(query)),
            Mode::Text => (command::TEXT, self.text.targets(query)),
        };
        entries.extend(command::filter(table, query));
        entries
    }

    fn open_palette(&self) {
        self.swatches.close();
        if let Err(error) = self.palette.open(self.entries("")) {
            web_sys::console::error_1(&error);
        }
    }

    fn close_palette(&self) {
        self.palette.close();
        let _ = self.show(self.mode.get());
    }

    /// Run whatever the palette (or a toolbar button, or a shortcut) asked for.
    ///
    /// The chrome's own verbs are answered here and everything else goes to the pane that is
    /// showing — so a command id is the one vocabulary a button, a key and a palette row all
    /// speak, and there is no second path for any of them to drift down.
    fn run(self: &Rc<Self>, id: &str) {
        match id {
            "doc.open" => self.open_picker(),
            "doc.save" => self.save(),
            "doc.undo" => self.undo(),
            "doc.redo" => self.redo(),
            "edit.copy" => self.copy_out(false),
            "edit.cut" => self.copy_out(true),
            "edit.paste" => self.paste_in(),
            _ => match self.mode.get() {
                Mode::Sheet => self.sheet.run(id),
                Mode::Text => self.text.run(id),
            },
        }
    }

    // --- the clipboard ---

    /// What the showing pane would put on the clipboard, as plain text. `None` when there is
    /// nothing to copy.
    fn clipboard_text(&self) -> Option<String> {
        match self.mode.get() {
            Mode::Sheet => self.sheet.clipboard_text(),
            Mode::Text => self.text.clipboard_text(),
        }
    }

    fn paste_text(&self, text: &str) {
        match self.mode.get() {
            Mode::Sheet => self.sheet.paste_text(text),
            Mode::Text => self.text.paste_text(text),
        }
    }

    fn delete_selection(&self) {
        match self.mode.get() {
            Mode::Sheet => self.sheet.run("edit.clear"),
            Mode::Text => {
                self.text.erase_selection();
            }
        }
    }

    /// Copy — or cut — from a *command* rather than from the browser's own event.
    ///
    /// `navigator.clipboard.writeText` needs the user gesture it is running inside, which a
    /// palette row and a toolbar button both are. The `copy`/`cut` events handle Ctrl+C and
    /// Ctrl+X without any of this; both paths end in the same two calls.
    fn copy_out(&self, cut: bool) {
        let Some(text) = self.clipboard_text() else {
            return self.set_message("Nothing to copy".to_owned());
        };
        let Some(clipboard) = clipboard() else {
            return self.set_message("Press Ctrl+C to copy".to_owned());
        };
        let _ = clipboard.write_text(&text);
        if cut {
            self.delete_selection();
        }
        self.set_message(match cut {
            true => "Cut".to_owned(),
            false => "Copied".to_owned(),
        });
    }

    /// Paste from a command. Reading the clipboard is the half browsers guard: it may prompt,
    /// and in some it is not there at all — so a failure says which key does work rather than
    /// leaving the command looking broken.
    fn paste_in(self: &Rc<Self>) {
        let Some(clipboard) = clipboard() else {
            return self.set_message("Press Ctrl+V to paste".to_owned());
        };
        let shell = self.clone();
        spawn_local(async move {
            match JsFuture::from(clipboard.read_text()).await {
                Ok(text) => shell.paste_text(&text.as_string().unwrap_or_default()),
                Err(_) => shell.set_message("Press Ctrl+V to paste".to_owned()),
            }
        });
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

/// Run a command from a pane — the same path a palette row and a toolbar button take.
///
/// A pane cannot hold an `Rc<Shell>` (a cycle that never drops), and `Shell::run` needs one
/// because pasting is asynchronous, so this reaches the live shell and hands the id over.
pub(crate) fn run_command(id: &str) {
    let shell = SHELL.with(|slot| slot.borrow().clone());
    if let Some(shell) = shell {
        shell.run(id);
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

/// `navigator.clipboard`, if this browser has one.
///
/// Looked up by property rather than through `Navigator::clipboard`, which assumes it is
/// there — it is absent in some browsers and in every headless runtime, and calling a method
/// on `undefined` would throw out of wasm rather than come back as `None`.
fn clipboard() -> Option<web_sys::Clipboard> {
    let navigator = web_sys::window()?.navigator();
    let value = js_sys::Reflect::get(&navigator, &JsValue::from_str("clipboard")).ok()?;
    (!value.is_undefined() && !value.is_null()).then(|| value.unchecked_into())
}

/// Show a toolbar toggle as pressed or not.
///
/// `aria-pressed` rather than a class: it is what a screen reader reads, the stylesheet keys
/// off the same attribute, and there is then one answer to "is this on" rather than two.
pub(crate) fn set_pressed(document: &Document, id: &str, on: bool) {
    if let Some(button) = document.get_element_by_id(id) {
        let _ = button.set_attribute("aria-pressed", if on { "true" } else { "false" });
    }
}

/// Show the colour a swatch button would apply, in the bar under its glyph. `None` paints the
/// page's own text colour, which is what "no colour of its own" looks like.
pub(crate) fn set_swatch(document: &Document, id: &str, color: Option<&str>) {
    if let Some(bar) = document.get_element_by_id(id) {
        let _ = bar.set_attribute(
            "style",
            &match color {
                Some(hex) => format!("background:{hex}"),
                None => "background:currentColor".to_owned(),
            },
        );
    }
}

/// Point a `<select>` at one of its options, without firing `change` — this is the toolbar
/// *reporting* what the selection already is, not asking for it to change.
pub(crate) fn set_select(document: &Document, id: &str, value: &str) {
    if let Some(select) = document
        .get_element_by_id(id)
        .and_then(|e| e.dyn_into::<web_sys::HtmlSelectElement>().ok())
        && select.value() != value
    {
        select.set_value(value);
    }
}

pub(crate) fn element<T: JsCast>(document: &Document, id: &str) -> Result<T, JsValue> {
    document
        .get_element_by_id(id)
        .ok_or_else(|| JsValue::from_str(&format!("index.html is missing #{id}")))?
        .dyn_into::<T>()
        .map_err(|_| JsValue::from_str(&format!("#{id} is not the element this shell expects")))
}

/// Which ODF form a name asks for — the core's rule, not a second one.
///
/// This used to spell the extension list itself, which meant `doc/flat-first.md` would have had
/// to be implemented twice and could have been implemented differently. A browser has no
/// filesystem (rule 5), but it does have `Path`, and a download's file *name* is exactly the
/// input `Form::from_path` takes.
pub(crate) fn form_of(name: &str) -> Form {
    Form::from_path(std::path::Path::new(name))
}

pub(crate) fn js(error: impl std::fmt::Display) -> JsValue {
    JsValue::from_str(&error.to_string())
}

// --- wiring ---

/// Every button in the chrome, and every button in either tool row.
///
/// **All of them run a command id.** A button is a way to say a verb, never a second
/// implementation of one — which is what keeps the toolbar, the palette and the keyboard
/// from drifting apart, and what makes `every_button_names_a_command` checkable.
fn wire_toolbar(shell: &Rc<Shell>) -> Result<(), JsValue> {
    // `(element id, command id, does it keep the focus)`. Opening is the one that keeps it:
    // the focus belongs in the file picker it just raised. Everything else hands the keyboard
    // straight back, or the next keystroke goes to a button instead of the document.
    const BUTTONS: &[(&str, &str, bool)] = &[
        ("open", "doc.open", true),
        ("save", "doc.save", false),
        ("undo", "doc.undo", false),
        ("redo", "doc.redo", false),
        ("recalc", "sheet.recalc", false),
        ("sheet-add", "sheet.add", false),
        ("s-bold", "style.bold", false),
        ("s-italic", "style.italic", false),
        ("s-align-left", "style.align-left", false),
        ("s-align-center", "style.align-center", false),
        ("s-align-right", "style.align-right", false),
        ("s-wrap", "style.wrap", false),
        ("s-border", "style.border", false),
        ("s-fewer", "format.fewer", false),
        ("s-more", "format.more", false),
        ("t-bold", "char.bold", false),
        ("t-italic", "char.italic", false),
        ("t-underline", "char.underline", false),
        ("t-strike", "char.strike", false),
        ("t-clear", "char.clear", false),
    ];
    for (id, command, keeps_focus) in BUTTONS {
        let button: HtmlButtonElement = element(&shell.dom.document, id)?;
        // **`click`, not `mousedown`.** A `mousedown` handler is unreachable from the
        // keyboard — Enter and Space on a focused button fire `click` and nothing else — and
        // this row has to work without a pointer.
        let shell = shell.clone();
        let command = (*command).to_owned();
        let keeps_focus = *keeps_focus;
        listen(&button, "click", move |_: Event| {
            shell.swatches.close();
            shell.run(&command);
            if !keeps_focus {
                let _ = shell.show(shell.mode.get());
            }
        })?;
        // Refusing the *press* is what keeps the button from taking the focus off the
        // document first, which is the reason `mousedown` was tempting above.
        keep_focus(&button)?;
    }

    // The two `<select>`s are the same thing with more than two states, so they run the
    // command their chosen option names.
    for id in ["s-format", "t-block"] {
        let select: HtmlElement = element(&shell.dom.document, id)?;
        let shell = shell.clone();
        listen(&select, "change", move |event: Event| {
            let Some(select) = event
                .target()
                .and_then(|t| t.dyn_into::<web_sys::HtmlSelectElement>().ok())
            else {
                return;
            };
            shell.run(&select.value());
            let _ = shell.show(shell.mode.get());
        })?;
    }

    // The palette's own button is not a command: it opens the list of them.
    Ok(())
}

/// The palette: opening it, typing in it, and choosing a row.
///
/// Its own keys are handled here rather than in a keymap because they are the palette's, not
/// the document's — a list has Up, Down, Enter and Escape whatever is in it.
fn wire_palette(shell: &Rc<Shell>) -> Result<(), JsValue> {
    let open = shell.clone();
    listen(&shell.dom.palette_open, "click", move |_: Event| {
        open.open_palette();
    })?;

    // Ctrl+K from anywhere, including from inside the panes' own key handlers, because this
    // listens on the document in the capture phase — a palette a pane could swallow would be
    // a palette that works in some places and not others.
    let keys = shell.clone();
    let listener = Closure::wrap(Box::new(move |event: KeyboardEvent| {
        let primary = event.ctrl_key() || event.meta_key();
        match event.key().as_str() {
            "k" | "K" if primary => {
                event.prevent_default();
                event.stop_propagation();
                match keys.palette.is_open() {
                    true => keys.close_palette(),
                    false => keys.open_palette(),
                }
            }
            "Escape" if keys.palette.is_open() => {
                event.prevent_default();
                event.stop_propagation();
                keys.close_palette();
            }
            "Escape" if keys.swatches.is_open() => keys.swatches.close(),
            _ => {}
        }
    }) as Box<dyn FnMut(KeyboardEvent)>);
    shell
        .dom
        .document
        .add_event_listener_with_callback_and_bool(
            "keydown",
            listener.as_ref().unchecked_ref(),
            true,
        )?;
    listener.forget();

    let typed = shell.clone();
    listen(&shell.palette.input, "input", move |_: Event| {
        let entries = typed.entries(&typed.palette.query());
        if let Err(error) = typed.palette.show(entries) {
            web_sys::console::error_1(&error);
        }
    })?;

    let moved = shell.clone();
    listen(
        &shell.palette.input,
        "keydown",
        move |event: KeyboardEvent| {
            let step = match event.key().as_str() {
                "ArrowDown" => 1,
                "ArrowUp" => -1,
                "Enter" => {
                    event.prevent_default();
                    if let Some(id) = moved.palette.chosen() {
                        moved.close_palette();
                        moved.run(&id);
                    }
                    return;
                }
                _ => return,
            };
            event.prevent_default();
            let _ = moved.palette.step(step);
        },
    )?;

    // Pointing at the palette: hovering picks, pressing runs. `mousedown` rather than `click`,
    // so the row does not have to survive the input losing focus first.
    let list: HtmlElement = element(&shell.dom.document, "palette-list")?;
    let over = shell.clone();
    listen(&list, "mousemove", move |event: MouseEvent| {
        if let Some(index) = row_index(&event) {
            let _ = over.palette.pick(index);
        }
    })?;
    let hit = shell.clone();
    listen(&list, "mousedown", move |event: MouseEvent| {
        let Some(index) = row_index(&event) else {
            return;
        };
        event.prevent_default();
        let _ = hit.palette.pick(index);
        if let Some(id) = hit.palette.chosen() {
            hit.close_palette();
            hit.run(&id);
        }
    })?;

    // Clicking the backdrop closes it, which is what every overlay on the web does.
    let away = shell.clone();
    listen(&shell.palette.input, "blur", move |_: Event| {
        // Only when the pointer went somewhere else entirely — a click *inside* the list
        // refocuses through the handlers above.
        if away.palette.is_open() {
            away.close_palette();
        }
    })
}

fn row_index(event: &MouseEvent) -> Option<usize> {
    let target = event.target()?.dyn_into::<Element>().ok()?;
    target
        .closest("li")
        .ok()??
        .get_attribute("data-index")?
        .parse()
        .ok()
}

/// The colour grid: which button opens it, and what a pick means.
fn wire_swatches(shell: &Rc<Shell>) -> Result<(), JsValue> {
    let owner = shell.clone();
    swatch::wire(&shell.swatches, move |target, hex| {
        match owner.mode.get() {
            Mode::Sheet => owner.sheet.set_color(&target, hex),
            Mode::Text => owner.text.set_color(&target, hex),
        }
        let _ = owner.show(owner.mode.get());
    })?;

    // Every swatch button in either tool row, and which property its grid sets.
    for (id, target) in [
        ("s-color", "color"),
        ("s-fill", "fill"),
        ("t-color", "color"),
        ("t-highlight", "highlight"),
    ] {
        let button: HtmlButtonElement = element(&shell.dom.document, id)?;
        let shell = shell.clone();
        let target = target.to_owned();
        let anchor = button.clone();
        listen(&button, "click", move |_: Event| {
            match shell.swatches.target().as_deref() == Some(target.as_str()) {
                true => shell.swatches.close(),
                false => {
                    if let Err(error) = shell.swatches.open(&anchor, target.clone()) {
                        web_sys::console::error_1(&error);
                    }
                }
            }
        })?;
        keep_focus(&button)?;
    }
    Ok(())
}

/// Stop a button taking the focus when it is pressed with the pointer.
///
/// The document being edited has to keep the keyboard: a toolbar that steals it leaves the
/// user typing into a button. Refusing `mousedown`'s default is the web's way to say so, and
/// it leaves `click` — and therefore the keyboard — untouched.
fn keep_focus(button: &HtmlButtonElement) -> Result<(), JsValue> {
    listen(button, "mousedown", move |event: MouseEvent| {
        event.prevent_default();
    })
}

/// Copy, cut and paste through the browser's own events.
///
/// **This is the path Ctrl+C actually takes.** `navigator.clipboard` needs a permission a
/// `copy` event does not, so the keys work everywhere and the palette's own commands are the
/// ones that fall back. Listened for on the document, because the event fires at whatever has
/// the focus and both panes are `tabindex` divs rather than fields.
fn wire_clipboard(shell: &Rc<Shell>, document: &Document) -> Result<(), JsValue> {
    // An edit in progress belongs to its `<input>`: the browser's own copy of a selected
    // formula is right, and ours would replace it with the cell underneath.
    let editing = |shell: &Shell| shell.mode.get() == Mode::Sheet && shell.sheet.is_editing();

    let copy = shell.clone();
    listen(document, "copy", move |event: ClipboardEvent| {
        if editing(&copy) || copy.palette.is_open() {
            return;
        }
        let (Some(data), Some(text)) = (event.clipboard_data(), copy.clipboard_text()) else {
            return;
        };
        event.prevent_default();
        let _ = data.set_data("text/plain", &text);
    })?;

    let cut = shell.clone();
    listen(document, "cut", move |event: ClipboardEvent| {
        if editing(&cut) || cut.palette.is_open() {
            return;
        }
        let (Some(data), Some(text)) = (event.clipboard_data(), cut.clipboard_text()) else {
            return;
        };
        event.prevent_default();
        let _ = data.set_data("text/plain", &text);
        cut.delete_selection();
    })?;

    let paste = shell.clone();
    listen(document, "paste", move |event: ClipboardEvent| {
        if editing(&paste) || paste.palette.is_open() {
            return;
        }
        let Some(data) = event.clipboard_data() else {
            return;
        };
        let Ok(text) = data.get_data("text/plain") else {
            return;
        };
        if text.is_empty() {
            return;
        }
        event.prevent_default();
        paste.paste_text(&text);
    })
}

/// Dropping a document on the page opens it — the gesture a browser has and a file dialog is
/// the long way round. The same bytes reach `Shell::open` either way.
fn wire_dropping(shell: &Rc<Shell>, document: &Document) -> Result<(), JsValue> {
    let over = shell.clone();
    listen(document, "dragover", move |event: DragEvent| {
        // Without this the browser navigates to the file, which loses the page.
        event.prevent_default();
        over.dom.drop.set_hidden(false);
    })?;

    let left = shell.clone();
    listen(document, "dragleave", move |event: DragEvent| {
        // Only when the pointer left the *page*, not one element inside it.
        if event.related_target().is_none() {
            left.dom.drop.set_hidden(true);
        }
    })?;

    let dropped = shell.clone();
    listen(document, "drop", move |event: DragEvent| {
        event.prevent_default();
        dropped.dom.drop.set_hidden(true);
        let Some(file) = event
            .data_transfer()
            .and_then(|data| data.files())
            .and_then(|files| files.get(0))
        else {
            return;
        };
        spawn_local(dropped.clone().load(file));
    })
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
        // A name that asks for nothing gets the form that diffs — `doc/flat-first.md`, decided
        // once in `Form::from_path` and reached from here rather than restated.
        assert_eq!(form_of("book"), Form::Flat);
    }
}
