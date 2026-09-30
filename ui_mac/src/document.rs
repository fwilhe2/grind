// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The document — an `NSDocument` holding an `App` (decision 5) — and the document controller that
//! decides which of the two kinds a file is.
//!
//! AppKit's document architecture supplies most of what "a Mac document application" means with
//! no code here: Open, Open Recent, Close, the window list, Revert, Duplicate, Rename, Move To and
//! the Versions browser all go to `NSDocument` and `NSDocumentController` by their standard
//! selectors (`menu.rs`). What is this shell's is small and said once:
//!
//! - **The bytes decide the kind, not the name.** `Controller` answers
//!   `typeForContentsOfURL:error:` by reading the file — `grind_core::kind`, and the workbook and
//!   CSV sniffs — which is the rule every shell follows and the reason a `.xml` holding a flat
//!   spreadsheet opens as one.
//! - **An import is untitled.** A workbook or a CSV is read to flat ODF and its document is left
//!   with no file URL, so neither Save nor, from M4, autosave has anything to write *into*. One
//!   way in, never out, kept by construction.
//! - **Undo is the core's** (rule 2). M4 answers `undo:` and `redo:` by forwarding to `App`; M2 is
//!   read-only, so autosave is off until then.
//!
//! The type names are the system's own identifiers for the two ODF kinds; which **form** a
//! document is saved in comes from its file name (`Form::from_path`), flat when it has none, which
//! is `doc/flat-first.md`'s rule.

use grind_core::DocumentKind;

/// The document type names this shell reads and writes.
pub const SHEET: &str = "org.oasis-open.opendocument.spreadsheet";
pub const TEXT: &str = "org.oasis-open.opendocument.text";

/// The type name a document kind is known by, when this shell opens that kind.
pub fn type_name(kind: DocumentKind) -> Option<&'static str> {
    match kind {
        DocumentKind::Spreadsheet => Some(SHEET),
        DocumentKind::Text => Some(TEXT),
        DocumentKind::Presentation => None,
    }
}

/// The kind a type name is.
pub fn kind_of(type_name: &str) -> Option<DocumentKind> {
    match type_name {
        SHEET => Some(DocumentKind::Spreadsheet),
        TEXT => Some(DocumentKind::Text),
        _ => None,
    }
}

/// The kind some bytes are, as the controller answers it: a workbook or a CSV is a spreadsheet
/// this shell imports, and anything else is what `grind_core::kind` reads.
pub fn sniff(name: &str, bytes: &[u8]) -> Result<DocumentKind, String> {
    if crate::import::is_workbook(bytes)
        || crate::import::is_delimited(std::path::Path::new(name), bytes)
    {
        return Ok(DocumentKind::Spreadsheet);
    }
    match grind_core::kind(bytes) {
        Some(DocumentKind::Presentation) => Err(format!(
            "{name} is a presentation, and this build opens spreadsheets and text documents"
        )),
        Some(kind) => Ok(kind),
        None => Err(format!("{name} is not an ODF spreadsheet or text document")),
    }
}

#[cfg(target_os = "macos")]
pub use mac::{Controller, Document};

#[cfg(target_os = "macos")]
mod mac {
    use std::cell::RefCell;
    use std::path::Path;
    use std::rc::Rc;
    use std::sync::Arc;

    use grind_core::{DocumentKind, Form};
    use objc2::rc::{Allocated, Retained, Weak as ObjcWeak};
    use objc2::runtime::{AnyClass, AnyObject};
    use objc2::{
        ClassType, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send,
    };
    use objc2_app_kit::{
        NSBackingStoreType, NSDocument, NSDocumentChangeType, NSDocumentController, NSOpenPanel,
        NSTextField, NSView, NSWindow, NSWindowController, NSWindowStyleMask,
    };
    use objc2_foundation::{
        NSArray, NSCocoaErrorDomain, NSData, NSDictionary, NSError, NSInteger,
        NSLocalizedDescriptionKey, NSPoint, NSRect, NSSize, NSString, NSURL,
    };

    use super::{SHEET, TEXT, kind_of, sniff};
    use crate::formatting::Formats;
    use crate::grid_view::{Pane, sheet_view};
    use crate::import;
    use crate::page_view::{self, TextPane, page_view};
    use crate::sidebar::{Places, Sidebar};
    use crate::toolbar::{self, Toolbar};
    use crate::{accessory, banner, find_bar, grid_view, sidebar};

    /// What a document holds: one of the suite's two `App`s.
    pub enum Content {
        Sheet(Arc<grind_sheet::App>),
        Text(Arc<grind_text::App>),
    }

    impl Content {
        fn open(name: &str, bytes: &[u8]) -> Result<Content, String> {
            let fail = |error: &dyn std::fmt::Display| format!("{name}: {error}");
            match grind_core::kind(bytes) {
                Some(DocumentKind::Spreadsheet) => {
                    let app = grind_sheet::App::new();
                    app.open_bytes(name, bytes).map_err(|e| fail(&e))?;
                    Ok(Content::Sheet(Arc::new(app)))
                }
                Some(DocumentKind::Text) => {
                    let app = grind_text::App::new();
                    app.open_bytes(name, bytes).map_err(|e| fail(&e))?;
                    Ok(Content::Text(Arc::new(app)))
                }
                _ => Err(format!("{name} is not a document this build opens")),
            }
        }

        fn save(&self, form: Form) -> Result<Vec<u8>, String> {
            match self {
                Content::Sheet(app) => app.save_bytes(form).map_err(|e| e.to_string()),
                Content::Text(app) => app.save_bytes(form).map_err(|e| e.to_string()),
            }
        }
    }

    #[derive(Default)]
    pub struct State {
        content: RefCell<Option<Content>>,
        /// The spreadsheet's views' shared state, once its window exists.
        pane: RefCell<Option<Rc<Pane>>>,
        /// The text document's page, once its window exists.
        page: RefCell<Option<Rc<TextPane>>>,
        /// The toolbar's delegate, which the toolbar holds only weakly.
        toolbar: RefCell<Option<Retained<Toolbar>>>,
        /// The sidebar's list, which its table holds only weakly.
        sidebar: RefCell<Option<Retained<Sidebar>>>,
        /// For an import: the ODF name it would be saved under, and the report's sentence.
        imported: RefCell<Option<(String, Option<String>)>>,
    }

    /// Write `message` into an `NSError **` a method was handed, if it was handed one.
    fn set_error(error: *mut *mut NSError, message: &str) {
        if error.is_null() {
            return;
        }
        let description = NSString::from_str(message);
        let value: &AnyObject = &description;
        // SAFETY: the key is a Foundation constant.
        let key: &NSString = unsafe { NSLocalizedDescriptionKey };
        let info = NSDictionary::<NSString, AnyObject>::from_slices(&[key], &[value]);
        // SAFETY: the domain is a Foundation constant, and a dictionary of strings is a valid
        // user-info dictionary.
        let made =
            unsafe { NSError::errorWithDomain_code_userInfo(NSCocoaErrorDomain, 0, Some(&info)) };
        // SAFETY: `error` is a valid out-pointer the caller owns, and an out-parameter error is
        // returned autoreleased by convention.
        unsafe { *error = Retained::autorelease_ptr(made) };
    }

    fn rect(x: f64, y: f64, w: f64, h: f64) -> NSRect {
        NSRect::new(NSPoint::new(x, y), NSSize::new(w, h))
    }

    define_class!(
        /// One open document, in one window.
        #[unsafe(super(NSDocument))]
        #[thread_kind = MainThreadOnly]
        #[name = "GrindDocument"]
        #[ivars = State]
        pub struct Document;

        impl Document {
            #[unsafe(method_id(init))]
            fn init(this: Allocated<Self>) -> Retained<Self> {
                let this = this.set_ivars(State::default());
                // SAFETY: `init` is `NSDocument`'s designated initialiser.
                unsafe { msg_send![super(this), init] }
            }

            /// Read the file itself rather than taking its bytes, because an import needs the name
            /// the bytes came under.
            #[unsafe(method(readFromURL:ofType:error:))]
            fn read_from_url(
                &self,
                url: &NSURL,
                _type_name: &NSString,
                error: *mut *mut NSError,
            ) -> bool {
                match self.read(url) {
                    Ok(()) => true,
                    Err(message) => {
                        set_error(error, &message);
                        false
                    }
                }
            }

            #[unsafe(method_id(dataOfType:error:))]
            fn data_of_type(
                &self,
                _type_name: &NSString,
                error: *mut *mut NSError,
            ) -> Option<Retained<NSData>> {
                // The form the file name asks for, and the flat form when there is none — an
                // untitled document, or an import — which is `doc/flat-first.md`'s default.
                let form = self
                    .fileURL()
                    .and_then(|url| url.to_file_path())
                    .map_or(Form::Flat, |path| Form::from_path(&path));
                let saved = match self.ivars().content.borrow().as_ref() {
                    Some(content) => content.save(form),
                    None => Err("nothing is open".to_owned()),
                };
                match saved {
                    Ok(bytes) => Some(NSData::with_bytes(&bytes)),
                    Err(message) => {
                        set_error(error, &message);
                        None
                    }
                }
            }

            #[unsafe(method(makeWindowControllers))]
            fn make_window_controllers(&self) {
                let mtm = self.mtm();
                let window = window(mtm);
                match self.ivars().content.borrow().as_ref() {
                    Some(Content::Sheet(app)) => {
                        let pane = Pane::new(app.clone());
                        let size = window.frame().size;
                        let scroll = sheet_view(&pane, size, mtm);
                        let places: Rc<dyn Places> = pane.clone();
                        let (split, list) = sidebar::split(places, &scroll, size.height, mtm);
                        *self.ivars().sidebar.borrow_mut() = Some(list);
                        window.setContentViewController(Some(&split));
                        // A content view controller sizes the window to its view; this is the size
                        // the window was made at.
                        window.setContentSize(size);
                        window.center();
                        // The grid takes the keyboard from the start, as a sheet's cursor does.
                        let grid = scroll.documentView();
                        window.makeFirstResponder(grid.as_deref().map(|view| &**view));
                        accessory::attach(&window, &pane, mtm);
                        pane.set_banner(banner::attach(&window, &pane, mtm));
                        pane.set_find_bar(find_bar::attach(&window, &pane, mtm));
                        // Every change the core reports marks the document edited, which is what
                        // the Edited dot and the autosave timer read. An open is not a change: the
                        // observer is attached after the document was read.
                        let me = ObjcWeak::new(self);
                        pane.on_change(move || {
                            if let Some(document) = me.load() {
                                document.updateChangeCount(NSDocumentChangeType::ChangeDone);
                            }
                        });
                        grid_view::watch(&pane);
                        let formats: Rc<dyn Formats> = pane.clone();
                        *self.ivars().toolbar.borrow_mut() = Some(toolbar::attach(
                            &window,
                            DocumentKind::Spreadsheet,
                            formats,
                            mtm,
                        ));
                        *self.ivars().pane.borrow_mut() = Some(pane);
                    }
                    Some(Content::Text(app)) => {
                        let pane = TextPane::new(app.clone());
                        let size = window.frame().size;
                        let scroll = page_view(&pane, size, mtm);
                        let places: Rc<dyn Places> = pane.clone();
                        let (split, list) = sidebar::split(places, &scroll, size.height, mtm);
                        *self.ivars().sidebar.borrow_mut() = Some(list);
                        window.setContentViewController(Some(&split));
                        window.setContentSize(size);
                        window.center();
                        // The page takes the keyboard from the start, caret at the top.
                        let page = scroll.documentView();
                        window.makeFirstResponder(page.as_deref().map(|view| &**view));
                        let me = ObjcWeak::new(self);
                        pane.on_change(move || {
                            if let Some(document) = me.load() {
                                document.updateChangeCount(NSDocumentChangeType::ChangeDone);
                            }
                        });
                        page_view::watch(&pane);
                        let formats: Rc<dyn Formats> = pane.clone();
                        *self.ivars().toolbar.borrow_mut() =
                            Some(toolbar::attach(&window, DocumentKind::Text, formats, mtm));
                        *self.ivars().page.borrow_mut() = Some(pane);
                    }
                    None => {
                        let label = NSTextField::labelWithString(
                            &NSString::from_str("Nothing is open."),
                            mtm,
                        );
                        window.setContentView(Some(&label));
                    }
                }
                if let Some((_, Some(summary))) = self.ivars().imported.borrow().as_ref() {
                    window.setSubtitle(&NSString::from_str(summary));
                }
                let controller =
                    NSWindowController::initWithWindow(NSWindowController::alloc(mtm), Some(&window));
                self.addWindowController(&controller);
            }

            /// An import is untitled, but not "Untitled": it goes by the ODF name it would be
            /// saved under, which is what Save offers too.
            #[unsafe(method_id(displayName))]
            fn display_name(&self) -> Retained<NSString> {
                match self.imported_name() {
                    Some(name) => NSString::from_str(&name),
                    // SAFETY: `displayName` is `NSDocument`'s own, with this signature.
                    None => unsafe { msg_send![super(self), displayName] },
                }
            }

            /// **Autosave in place** — the Mac convention since 10.7, and the user's choice
            /// (decision 5). Safe here for three reasons the decision gives: an untouched document
            /// is never written, since only a change the core reports marks it edited; an import
            /// is untitled, so its autosave goes to `~/Library/Autosave Information` and never
            /// beside the workbook; and every save is `App::save_bytes`, so R6's splicing makes an
            /// autosave one line of diff like any other save.
            #[unsafe(method(autosavesInPlace))]
            fn autosaves_in_place() -> bool {
                true
            }

            /// No `NSUndoManager`: undo is the core's (architecture rule 2), and the grid answers
            /// `undo:` and `redo:` itself.
            #[unsafe(method(hasUndoManager))]
            fn has_undo_manager(&self) -> bool {
                false
            }

            #[unsafe(method_id(readableTypes))]
            fn readable_types() -> Retained<NSArray<NSString>> {
                NSArray::from_retained_slice(&[NSString::from_str(SHEET), NSString::from_str(TEXT)])
            }

            #[unsafe(method_id(writableTypes))]
            fn writable_types() -> Retained<NSArray<NSString>> {
                NSArray::from_retained_slice(&[NSString::from_str(SHEET), NSString::from_str(TEXT)])
            }

            #[unsafe(method(isNativeType:))]
            fn is_native_type(type_name: &NSString) -> bool {
                kind_of(&type_name.to_string()).is_some()
            }
        }
    );

    impl Document {
        fn read(&self, url: &NSURL) -> Result<(), String> {
            let path = url.to_file_path().ok_or("only a file can be opened")?;
            let name = path.display().to_string();
            let bytes = std::fs::read(&path).map_err(|error| format!("{name}: {error}"))?;
            let opened = import::open(&name, &bytes)?;
            let content = Content::open(&opened.name, &opened.bytes)?;
            *self.ivars().content.borrow_mut() = Some(content);
            *self.ivars().imported.borrow_mut() = opened
                .untitled
                .then(|| (opened.name.clone(), opened.summary.clone()));
            Ok(())
        }

        /// The file name an import goes by, which is the ODF name it would be saved under.
        fn imported_name(&self) -> Option<String> {
            let imported = self.ivars().imported.borrow();
            let (name, _) = imported.as_ref()?;
            Some(
                Path::new(name)
                    .file_name()
                    .map_or(name.clone(), |file| file.to_string_lossy().into_owned()),
            )
        }

        /// The spreadsheet's pane, when this document is one with a window.
        pub fn pane(&self) -> Option<Rc<Pane>> {
            self.ivars().pane.borrow().clone()
        }

        /// Which kind of document this is.
        pub fn kind(&self) -> Option<DocumentKind> {
            match self.ivars().content.borrow().as_ref()? {
                Content::Sheet(_) => Some(DocumentKind::Spreadsheet),
                Content::Text(_) => Some(DocumentKind::Text),
            }
        }

        /// A Format command, on whichever pane this document has (M7).
        pub fn format(&self, command: crate::menu::Command) {
            if let Some(pane) = self.pane() {
                pane.format(command);
            } else if let Some(page) = self.page() {
                page.format(command);
            }
        }

        /// View ▸ Cell Roles and Names, on whichever pane this document has.
        pub fn toggle_overlay(&self, command: crate::menu::Command) {
            let roles = command == crate::menu::Command::CellRoles;
            if let Some(pane) = self.pane() {
                pane.toggle_overlay(roles);
            } else if let (Some(page), false) = (self.page(), roles) {
                page.toggle_names();
            }
        }

        /// Whether a Format command's item is ticked here.
        pub fn format_checked(&self, command: crate::menu::Command) -> bool {
            match (self.pane(), self.page()) {
                (Some(pane), _) => pane.format_checked(command),
                (_, Some(page)) => page.format_checked(command),
                _ => false,
            }
        }

        /// The text document's page, when this document is one with a window.
        pub fn page(&self) -> Option<Rc<TextPane>> {
            self.ivars().page.borrow().clone()
        }

        /// The view a drive's click lands in — the grid or the page — and how far its own
        /// origin is from the one a script counts from: the grid keeps a margin the header
        /// bands float over, and a script's points are the sheet's.
        pub fn click_target(&self) -> Option<(Retained<NSView>, (f64, f64))> {
            if let Some(grid) = self.pane().and_then(|pane| pane.grid_view()) {
                let origin = (crate::sheet::geom::HEADER_W, crate::sheet::geom::HEADER_H);
                return Some((grid.into_super(), origin));
            }
            let page = self.page()?.view()?;
            Some((page.into_super(), (0.0, 0.0)))
        }

        /// Where the selection is, for a drive's transcript: the name box's word, and the range
        /// when there is more than one cell — or on a page, the caret's address.
        pub fn selection_text(&self) -> String {
            if let Some(page) = self.page() {
                return page.caret_text();
            }
            let Some(pane) = self.pane() else {
                return "no grid".to_owned();
            };
            let selection = pane.selection.get();
            let (start, end) = selection.rect();
            let named = grind_sheet::place::name_box(&pane.app, pane.sheet.get(), selection);
            match selection.is_single() {
                true => named,
                false => format!(
                    "{}:{} (active {named})",
                    grind_sheet::a1::format(None, start),
                    grind_sheet::a1::format(None, end)
                ),
            }
        }

        /// Whether this document came from a workbook or a CSV, and so must stay untitled.
        pub fn is_import(&self) -> bool {
            self.ivars().imported.borrow().is_some()
        }

        /// An empty document of `kind`, for New — `Document::default()` for either type, which is
        /// one empty sheet or one empty paragraph.
        pub fn start(&self, kind: DocumentKind) {
            let content = match kind {
                DocumentKind::Text => Content::Text(Arc::new(grind_text::App::new())),
                _ => Content::Sheet(Arc::new(grind_sheet::App::new())),
            };
            *self.ivars().content.borrow_mut() = Some(content);
        }
    }

    /// A document window: titled, closable, miniaturisable and resizable, and not released on
    /// close, since the window controller owns it.
    fn window(mtm: MainThreadMarker) -> Retained<NSWindow> {
        // SAFETY: a titled, buffered window is the ordinary kind.
        let window = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                NSWindow::alloc(mtm),
                rect(0.0, 0.0, 900.0, 600.0),
                NSWindowStyleMask::Titled
                    | NSWindowStyleMask::Closable
                    | NSWindowStyleMask::Miniaturizable
                    | NSWindowStyleMask::Resizable,
                NSBackingStoreType::Buffered,
                false,
            )
        };
        // SAFETY: the window is owned by Rust and its controller, so AppKit must not also
        // release it on close — objc2's documented rule for windows created from Rust.
        unsafe { window.setReleasedWhenClosed(false) };
        window.center();
        window
    }

    define_class!(
        /// The shared document controller — the first one made is the shared one, which is why
        /// `app.rs` makes it before anything asks for it.
        #[unsafe(super(NSDocumentController))]
        #[thread_kind = MainThreadOnly]
        #[name = "GrindDocumentController"]
        pub struct Controller;

        impl Controller {
            #[unsafe(method(documentClassForType:))]
            fn document_class_for_type(&self, _type_name: &NSString) -> &'static AnyClass {
                Document::class()
            }

            #[unsafe(method_id(documentClassNames))]
            fn document_class_names(&self) -> Retained<NSArray<NSString>> {
                NSArray::from_retained_slice(&[NSString::from_str("GrindDocument")])
            }

            #[unsafe(method_id(defaultType))]
            fn default_type(&self) -> Option<Retained<NSString>> {
                Some(NSString::from_str(SHEET))
            }

            /// The kind a file is, **read from its bytes** — never from its name.
            #[unsafe(method_id(typeForContentsOfURL:error:))]
            fn type_for_contents(
                &self,
                url: &NSURL,
                error: *mut *mut NSError,
            ) -> Option<Retained<NSString>> {
                type_of(url, error)
            }

            /// Opened as usual, and then an import is made untitled: it has no file of its own to
            /// be saved back to.
            #[unsafe(method_id(makeDocumentWithContentsOfURL:ofType:error:))]
            fn make_document(
                &self,
                url: &NSURL,
                type_name: &NSString,
                error: *mut *mut NSError,
            ) -> Option<Retained<NSDocument>> {
                // SAFETY: the superclass's own method, with its own signature.
                let document: Option<Retained<NSDocument>> = unsafe {
                    msg_send![super(self), makeDocumentWithContentsOfURL: url, ofType: type_name, error: error]
                };
                document.map(untitled_if_imported)
            }

            /// Every file, not only the types the system has heard of: a flat `.fods`, a
            /// projection or a CSV is declared by no framework, and the bytes decide anyway.
            #[unsafe(method(runModalOpenPanel:forTypes:))]
            fn run_modal_open_panel(
                &self,
                panel: &NSOpenPanel,
                _types: Option<&NSArray<NSString>>,
            ) -> NSInteger {
                let none: Option<&NSArray<NSString>> = None;
                // SAFETY: the superclass's own method; nil types means "any file".
                unsafe { msg_send![super(self), runModalOpenPanel: panel, forTypes: none] }
            }
        }
    );

    /// The type name of the file at `url`, read from its bytes, or an error said into `error`.
    fn type_of(url: &NSURL, error: *mut *mut NSError) -> Option<Retained<NSString>> {
        let kind = url
            .to_file_path()
            .ok_or_else(|| "only a file can be opened".to_owned())
            .and_then(|path| {
                let name = path.display().to_string();
                let bytes = std::fs::read(&path).map_err(|error| format!("{name}: {error}"))?;
                sniff(&name, &bytes)
            });
        match kind.map(super::type_name) {
            Ok(Some(name)) => Some(NSString::from_str(name)),
            Ok(None) => {
                set_error(error, "this build does not open that kind of document");
                None
            }
            Err(message) => {
                set_error(error, &message);
                None
            }
        }
    }

    /// `document`, with its file URL taken away when it is an import — so it is untitled, and
    /// has no file of its own to be saved back to.
    fn untitled_if_imported(document: Retained<NSDocument>) -> Retained<NSDocument> {
        if let Ok(ours) = document.clone().downcast::<Document>()
            && ours.is_import()
        {
            document.setFileURL(None);
        }
        document
    }

    impl Controller {
        /// Make the shared document controller. Called once, before anything asks for
        /// `NSDocumentController::sharedDocumentController`.
        pub fn install(mtm: MainThreadMarker) -> Retained<Controller> {
            // SAFETY: `init` is `NSDocumentController`'s designated initialiser, and this class
            // has no instance variables of its own to set.
            unsafe { msg_send![Controller::alloc(mtm), init] }
        }

        /// An empty document of `kind`, shown.
        pub fn new_document(&self, kind: DocumentKind) -> Result<(), String> {
            let type_name = super::type_name(kind).ok_or("not a kind this build makes")?;
            let document = self
                .makeUntitledDocumentOfType_error(&NSString::from_str(type_name))
                .map_err(|error| error.localizedDescription().to_string())?;
            if let Ok(ours) = document.clone().downcast::<Document>() {
                ours.start(kind);
            }
            self.addDocument(&document);
            document.makeWindowControllers();
            document.showWindows();
            Ok(())
        }

        /// The document at `path`, opened and shown — the same path File ▸ Open takes, minus
        /// the panel.
        pub fn open_path(&self, path: &Path) -> Result<Retained<NSDocument>, String> {
            let url = NSURL::from_file_path(path).ok_or("not a file path")?;
            let type_name = self
                .typeForContentsOfURL_error(&url)
                .map_err(|error| error.localizedDescription().to_string())?;
            let document = self
                .makeDocumentWithContentsOfURL_ofType_error(&url, &type_name)
                .map_err(|error| error.localizedDescription().to_string())?;
            self.addDocument(&document);
            document.makeWindowControllers();
            document.showWindows();
            Ok(document)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_two_kinds_have_a_type_name_each_and_back() {
        for kind in [DocumentKind::Spreadsheet, DocumentKind::Text] {
            let name = type_name(kind).unwrap();
            assert_eq!(kind_of(name), Some(kind));
        }
        assert_eq!(type_name(DocumentKind::Presentation), None);
        assert_eq!(kind_of("public.plain-text"), None);
    }

    /// The bytes decide: a text document under a spreadsheet's name is a text document.
    #[test]
    fn the_bytes_decide_the_kind() {
        let text =
            grind_text::write_bytes(&grind_text::Document::default(), grind_core::Form::Flat)
                .unwrap();
        assert_eq!(sniff("lying.fods", &text), Ok(DocumentKind::Text));
        assert_eq!(
            sniff("prices.csv", b"item,price\n"),
            Ok(DocumentKind::Spreadsheet),
            "a CSV is a spreadsheet this shell imports"
        );
        assert!(sniff("notes.txt", b"hello").is_err());
    }
}
