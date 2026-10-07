// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The application: `NSApplication`, its delegate, and the menu bar built from `menu.rs`.
//!
//! Everything the menu bar *is* was decided in `menu.rs`, portable and tested; this file turns
//! each row into an `NSMenuItem` — a standard selector for the responder chain, or
//! [`COMMAND_SELECTOR`] with the command's tag for the one thing the delegate answers itself — and
//! tells AppKit which menus are the Window, Help and Services menus. Documents are
//! `NSDocumentController`'s ([`Controller`]), which is made first so that it is the shared one.
//!
//! A launch with a file opens it the way File ▸ Open would; a launch with `--sheet` or `--text`
//! starts an empty document of that kind; and a launch with neither, which decision 6 answers with
//! the welcome window, starts an empty spreadsheet until M9 builds that window.

use std::cell::RefCell;
use std::ffi::CString;
use std::process::ExitCode;

use grind_core::DocumentKind;
use objc2::rc::{Retained, Weak};
use objc2::runtime::{AnyObject, ProtocolObject, Sel};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, Message, define_class, msg_send};
use objc2_app_kit::{
    NSAboutPanelOptionApplicationVersion, NSAboutPanelOptionCredits, NSAboutPanelOptionKey,
    NSAboutPanelOptionVersion, NSAlert, NSAlertFirstButtonReturn, NSApplication,
    NSApplicationActivationPolicy, NSApplicationDelegate, NSControlStateValueOff,
    NSControlStateValueOn, NSEventModifierFlags, NSMenu, NSMenuItem, NSPopUpButton, NSWindow,
};
use objc2_foundation::{
    NSAttributedString, NSDictionary, NSNotification, NSObject, NSObjectProtocol, NSPoint, NSRect,
    NSSize, NSString, NSUserDefaults,
};

use crate::Opening;
use crate::document::{Controller, Document};
use crate::menu::{self, Action, COMMAND_SELECTOR, Command, Item, MENUS, Menu, Role};
use crate::prompt;
use crate::welcome_window::{self, Picked};

/// What the delegate was launched to do, until it has done it.
pub struct Launch {
    opening: RefCell<Option<Opening>>,
    controller: Retained<Controller>,
    /// The welcome window, while it exists (decision 6).
    welcome: RefCell<Option<Retained<NSWindow>>>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "GrindAppDelegate"]
    #[ivars = Launch]
    pub struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}

    unsafe impl NSApplicationDelegate for Delegate {
        #[unsafe(method(applicationDidFinishLaunching:))]
        fn did_finish_launching(&self, _notification: &NSNotification) {
            if let Some(opening) = self.ivars().opening.borrow_mut().take() {
                self.launch(opening);
            }
        }

        /// No untitled document of AppKit's own at launch: what opens is the command line's
        /// decision, made in [`Delegate::launch`].
        #[unsafe(method(applicationShouldOpenUntitledFile:))]
        fn should_open_untitled_file(&self, _app: &NSApplication) -> bool {
            false
        }

        /// A Dock click with no window open brings the welcome window back (decision 6).
        #[unsafe(method(applicationShouldHandleReopen:hasVisibleWindows:))]
        fn should_handle_reopen(&self, _app: &NSApplication, visible: bool) -> bool {
            if !visible {
                self.show_welcome();
            }
            true
        }

        /// A document application keeps running with no windows, which is the Mac convention
        /// (decision 5).
        #[unsafe(method(applicationShouldTerminateAfterLastWindowClosed:))]
        fn should_terminate_after_last_window_closed(&self, _app: &NSApplication) -> bool {
            false
        }
    }

    impl Delegate {
        /// Every [`Command`] item's action, reached down the responder chain, with the command in
        /// the item's tag.
        #[unsafe(method(performGrindCommand:))]
        fn perform_grind_command(&self, sender: Option<&AnyObject>) {
            let command = sender
                .and_then(|sender| sender.downcast_ref::<NSMenuItem>())
                .and_then(|item| Command::from_tag(item.tag()));
            if let Some(command) = command {
                self.perform(command);
            }
        }

        /// A command is grey where it means nothing — Code over a spreadsheet, a number format
        /// over a page, anything but New with no document — and ticked where the selection
        /// already is what it would make it.
        #[unsafe(method(validateMenuItem:))]
        fn validate_menu_item(&self, item: &NSMenuItem) -> bool {
            // A method body here may not return early, so the work is a function's.
            self.validate(item)
        }
    }
);

impl Delegate {
    fn new(
        mtm: MainThreadMarker,
        opening: Opening,
        controller: Retained<Controller>,
    ) -> Retained<Self> {
        let this = Delegate::alloc(mtm).set_ivars(Launch {
            opening: RefCell::new(Some(opening)),
            controller,
            welcome: RefCell::new(None),
        });
        // SAFETY: `init` is `NSObject`'s designated initialiser.
        unsafe { msg_send![super(this), init] }
    }

    /// `validateMenuItem:`'s answer.
    fn validate(&self, item: &NSMenuItem) -> bool {
        let ours = item
            .action()
            .is_some_and(|action| action.name().to_str() == Ok(COMMAND_SELECTOR));
        let Some(command) = Command::from_tag(item.tag()).filter(|_| ours) else {
            return true;
        };
        let document = self.front_document();
        let applies = match document.as_ref().and_then(|document| document.kind()) {
            Some(kind) => command.applies(kind),
            None => matches!(
                command,
                Command::NewSheet | Command::NewText | Command::About
            ),
        };
        // A number format's item says what the active cell would look like in it.
        if matches!(command, Command::Number(_)) {
            let sample = document
                .as_ref()
                .filter(|_| applies)
                .and_then(|document| document.number_sample(command));
            item.setSubtitle(sample.map(|sample| NSString::from_str(&sample)).as_deref());
        }
        let on = applies && document.is_some_and(|document| document.format_checked(command));
        item.setState(match on {
            true => NSControlStateValueOn,
            false => NSControlStateValueOff,
        });
        applies
    }

    /// The whole of what a command does — matched exhaustively, so a command with no handler
    /// fails the build.
    fn perform(&self, command: Command) {
        let kind = match command {
            Command::NewSheet => DocumentKind::Spreadsheet,
            Command::NewText => DocumentKind::Text,
            // The rest act on the front document's grid, when the front document is a
            // spreadsheet; with none, they do nothing.
            // Formatting acts on whichever pane the front document has (M7).
            Command::Mark(_)
            | Command::Align(_)
            | Command::Indent(_)
            | Command::Wrap
            | Command::Borders(_)
            | Command::Number(_)
            | Command::Decimals(_)
            | Command::Currency(_)
            | Command::Group
            | Command::TextColor(_)
            | Command::Background(_)
            | Command::Block(_)
            | Command::ClearFormatting => {
                if let Some(document) = self.front_document() {
                    document.format(command);
                }
                return;
            }
            Command::Welcome => {
                self.show_welcome();
                return;
            }
            Command::About => {
                about(self.mtm());
                return;
            }
            Command::Zoom(step) => {
                if let Some(document) = self.front_document() {
                    document.zoom(step);
                }
                return;
            }
            Command::ShowSource => {
                if let Some(document) = self.front_document() {
                    document.toggle_source();
                }
                return;
            }
            // The overlays (M8), on either pane.
            Command::CellRoles | Command::Names => {
                if let Some(document) = self.front_document() {
                    document.toggle_overlay(command);
                }
                return;
            }
            // Formula literacy (M8), on the front document's grid.
            Command::FriendlyFormulas | Command::ExplainFormula | Command::InsertFunction => {
                if let Some(pane) = self.front_pane() {
                    match command {
                        Command::FriendlyFormulas => pane.toggle_friendly(),
                        Command::ExplainFormula => {
                            alert(self.mtm(), "Explain Formula", &pane.explanation())
                        }
                        _ => {
                            if let Some(at) = ask_function(self.mtm()) {
                                pane.insert_function(at);
                            }
                        }
                    }
                }
                return;
            }
            // Go To on a page: an address asked for, and the caret there.
            Command::GoTo if self.front_pane().is_none() => {
                if let Some(page) = self.front_document().and_then(|document| document.page())
                    && let Some(address) = ask_address(self.mtm())
                {
                    match crate::places::caret(&page.app, &address) {
                        Some(caret) => page.go_to(caret),
                        None => alert(
                            self.mtm(),
                            "There is no such place.",
                            &format!("“{address}” is not an address in this document."),
                        ),
                    }
                }
                return;
            }
            Command::Fill(_)
            | Command::Rows(_)
            | Command::Columns(_)
            | Command::FitAll
            | Command::DefineName
            | Command::InsertChart
            | Command::PreviewChart
            | Command::Filter
            | Command::CopyValue
            | Command::FormulaToValue
            | Command::Evaluate
            | Command::FillAcross
            | Command::Merge(_)
            | Command::Calculations
            | Command::DocumentLocale
            | Command::AddRule
            | Command::RemoveRule
            | Command::ExportCsv
            | Command::ImportCsvWith
            | Command::ImportCsv => {
                if let Some(pane) = self.front_pane() {
                    pane.structure(command, self.mtm());
                }
                return;
            }
            Command::InsertTable
            | Command::InsertBookmark
            | Command::InsertPicture
            | Command::ImportMarkdown
            | Command::ExportMarkdown
            | Command::ExportPdf
            | Command::Print
            | Command::MoveParagraph(_)
            | Command::DeleteParagraph
            | Command::ParagraphStyle => {
                if let Some(page) = self.front_document().and_then(|document| document.page()) {
                    page.structure(command, self.mtm());
                }
                return;
            }
            Command::ShowFormulas => {
                if let Some(pane) = self.front_pane() {
                    pane.toggle_formulas();
                }
                return;
            }
            Command::Recalculate => {
                if let Some(pane) = self.front_pane() {
                    pane.recalculate_anyway();
                }
                return;
            }
            Command::GoTo | Command::AddSheet | Command::RenameSheet | Command::DeleteSheet => {
                if let Some(pane) = self.front_pane() {
                    match command {
                        Command::GoTo => pane.focus_name_box(),
                        Command::AddSheet => pane.add_sheet(),
                        Command::RenameSheet => {
                            if let Some(name) = ask_sheet_name(self.mtm(), &pane.sheet_name()) {
                                pane.rename_sheet(&name);
                            }
                        }
                        _ => pane.delete_sheet(),
                    }
                }
                return;
            }
        };
        if let Err(message) = self.ivars().controller.new_document(kind) {
            alert(self.mtm(), "The document could not be made.", &message);
        }
    }

    /// The document a command acts on: `currentDocument` — the main window's — and when no
    /// window is main, the frontmost window's document, then the first there is.
    ///
    /// The fallback is not a nicety. A runner's application never becomes active, so no window
    /// is ever main there (*Evidence*), and `currentDocument` alone left every command a drive
    /// sent reaching nothing: Go To did not go.
    fn front_document(&self) -> Option<Retained<Document>> {
        let controller = &self.ivars().controller;
        let app = NSApplication::sharedApplication(self.mtm());
        controller
            .currentDocument()
            .or_else(|| {
                app.orderedWindows()
                    .iter()
                    .find_map(|window| controller.documentForWindow(&window))
            })
            .or_else(|| controller.documents().firstObject())
            .and_then(|document| document.downcast::<Document>().ok())
    }

    /// The front document's grid, when the front document is a spreadsheet with a window.
    fn front_pane(&self) -> Option<std::rc::Rc<crate::grid_view::Pane>> {
        self.front_document().and_then(|document| document.pane())
    }

    fn launch(&self, opening: Opening) {
        let mtm = self.mtm();
        if opening.drive.is_some() {
            crate::page_view::STEADY.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        let controller = &self.ivars().controller;
        let opened = match (&opening.path, opening.kind) {
            (Some(path), _) => controller.open_path(path).map(Some),
            // The document is handed to a drive like an opened one is, so its transcript can say
            // where the selection is and its `save` has something to save.
            (None, Some(kind)) => controller.new_document(kind).map(Some),
            // Nothing named: the choice, not a guess (decision 6).
            (None, None) => {
                self.show_welcome();
                Ok(None)
            }
        };
        let app = NSApplication::sharedApplication(mtm);
        match (opened, &opening.drive) {
            (Ok(document), Some(drive)) => {
                println!("launch: {}", crate::intent(&opening));
                let code = crate::drive::replay(mtm, drive, document.as_deref());
                std::process::exit(code);
            }
            (Err(message), Some(_)) => {
                eprintln!("grind-mac: {message}");
                std::process::exit(1);
            }
            (Err(message), None) => {
                alert(mtm, "The document could not be opened.", &message);
            }
            (Ok(_), None) => {}
        }
        app.activate();
    }
}

impl Delegate {
    /// The welcome window, in front: the one there is, or a new one listing the system's recent
    /// documents.
    fn show_welcome(&self) {
        let mtm = self.mtm();
        if let Some(window) = self.ivars().welcome.borrow().as_ref() {
            window.makeKeyAndOrderFront(None);
            return;
        }
        let recent: Vec<std::path::PathBuf> = self
            .ivars()
            .controller
            .recentDocumentURLs()
            .iter()
            .filter_map(|url| url.to_file_path())
            .take(crate::welcome::RECENT_MAX)
            .collect();
        let me = Weak::from_retained(&self.retain());
        let window = welcome_window::show(
            recent,
            move |picked| {
                let Some(delegate) = me.load() else { return };
                delegate.picked(picked);
            },
            mtm,
        );
        *self.ivars().welcome.borrow_mut() = Some(window);
    }

    /// A choice made in the welcome window: the menu item it is, run.
    fn picked(&self, picked: Picked) {
        *self.ivars().welcome.borrow_mut() = None;
        match picked {
            Picked::Choice(choice) => match choice.action() {
                Action::Command(command) => self.perform(command),
                // SAFETY: an action, and a nil sender is allowed.
                _ => unsafe { self.ivars().controller.openDocument(None) },
            },
            Picked::Recent(path) => {
                if let Err(message) = self.ivars().controller.open_path(&path) {
                    alert(self.mtm(), "The document could not be opened.", &message);
                }
            }
        }
    }
}

/// Say what went wrong.
fn alert(mtm: MainThreadMarker, message: &str, detail: &str) {
    prompt::tell(mtm, message, detail);
}

/// Ask for a place on a page — `p12`, `p12+40`, `#intro`, `§2.1.3`.
fn ask_address(mtm: MainThreadMarker) -> Option<String> {
    prompt::ask(
        mtm,
        "Go To",
        "A paragraph (p12), a place in one (p12+40), a bookmark (#intro) or an outline path (§2.1).",
        "Go",
        "",
    )
}

/// Ask for a sheet's new name, starting from `current`. `None` when it is cancelled.
fn ask_sheet_name(mtm: MainThreadMarker, current: &str) -> Option<String> {
    prompt::ask(
        mtm,
        "Rename Sheet",
        "Every formula, name and chart that refers to this sheet follows the new name.",
        "Rename",
        current,
    )
}

/// Insert ▸ Function…: every function this build has, by its name and its plain-English one,
/// in a pop-up under an alert — the platform's own shape for choosing one of many, with each
/// row's summary as its tooltip. The chosen row's place in the catalog, or `None`.
fn ask_function(mtm: MainThreadMarker) -> Option<usize> {
    let alert = NSAlert::new(mtm);
    alert.setMessageText(&NSString::from_str("Insert Function"));
    alert.setInformativeText(&NSString::from_str(
        "The call is written into the cell, ready for its arguments.",
    ));
    alert.addButtonWithTitle(&NSString::from_str("Insert"));
    alert.addButtonWithTitle(&NSString::from_str("Cancel"));
    let popup = NSPopUpButton::initWithFrame_pullsDown(
        NSPopUpButton::alloc(mtm),
        NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(320.0, 26.0)),
        false,
    );
    for info in grind_sheet::formula::funcs::catalog() {
        let alias = grind_sheet::formula::friendly::alias(info.name).unwrap_or(info.name);
        popup.addItemWithTitle(&NSString::from_str(&format!(
            "{} \u{2014} {alias}",
            info.name
        )));
        if let Some(item) = popup.lastItem() {
            item.setToolTip(Some(&NSString::from_str(info.brief)));
        }
    }
    alert.setAccessoryView(Some(&popup));
    (alert.runModal() == NSAlertFirstButtonReturn)
        .then(|| usize::try_from(popup.indexOfSelectedItem()).ok())
        .flatten()
}
/// A selector by name, as `menu.rs` spells it.
fn selector(name: &str) -> Sel {
    let name = CString::new(name).expect("a selector has no NUL in it");
    Sel::register(&name)
}

fn modifiers(mods: menu::Mods) -> NSEventModifierFlags {
    let mut flags = NSEventModifierFlags::empty();
    for (on, flag) in [
        (mods.command, NSEventModifierFlags::Command),
        (mods.shift, NSEventModifierFlags::Shift),
        (mods.option, NSEventModifierFlags::Option),
        (mods.control, NSEventModifierFlags::Control),
    ] {
        if on {
            flags |= flag;
        }
    }
    flags
}

/// One of `menu.rs`'s menus as an `NSMenu`, telling `app` which of its own menus it is.
fn build(menu: &'static Menu, app: &NSApplication, mtm: MainThreadMarker) -> Retained<NSMenu> {
    let built = NSMenu::initWithTitle(NSMenu::alloc(mtm), &NSString::from_str(menu.title));
    for item in menu.items {
        let made = match item {
            Item::Separator => NSMenuItem::separatorItem(mtm),
            Item::Submenu { title, menu } => {
                let made = NSMenuItem::new(mtm);
                made.setTitle(&NSString::from_str(title));
                made.setSubmenu(Some(&build(menu, app, mtm)));
                made
            }
            Item::Entry { title, key, action } => {
                let (sel, tag) = match action {
                    Action::Standard(name) => (selector(name), 0),
                    Action::Tagged(name, tag) => (selector(name), *tag),
                    Action::Command(command) => (selector(COMMAND_SELECTOR), command.tag()),
                };
                let equivalent = key.map_or("", |key| key.key);
                // SAFETY: the selector names an action method, which takes one sender argument.
                let made = unsafe {
                    NSMenuItem::initWithTitle_action_keyEquivalent(
                        NSMenuItem::alloc(mtm),
                        &NSString::from_str(title),
                        Some(sel),
                        &NSString::from_str(equivalent),
                    )
                };
                if let Some(key) = key {
                    made.setKeyEquivalentModifierMask(modifiers(key.mods));
                }
                made.setTag(tag);
                made
            }
        };
        built.addItem(&made);
    }
    match menu.role {
        Role::Window => app.setWindowsMenu(Some(&built)),
        Role::Help => app.setHelpMenu(Some(&built)),
        Role::Services => app.setServicesMenu(Some(&built)),
        Role::Application | Role::OpenRecent | Role::Plain => {}
    }
    built
}

/// A context menu (M9) from `menu.rs`'s table — built by the same function as the menu bar,
/// so a row and its bar twin are one kind of item.
pub fn context_menu(menu: &'static Menu, mtm: MainThreadMarker) -> Retained<NSMenu> {
    build(menu, &NSApplication::sharedApplication(mtm), mtm)
}

/// The whole menu bar: each of [`MENUS`] under a top-level item of its own.
fn menu_bar(app: &NSApplication, mtm: MainThreadMarker) -> Retained<NSMenu> {
    let bar = NSMenu::new(mtm);
    for menu in MENUS {
        let top = NSMenuItem::new(mtm);
        top.setTitle(&NSString::from_str(menu.title));
        top.setSubmenu(Some(&build(menu, app, mtm)));
        bar.addItem(&top);
    }
    bar
}

/// Run the application until it quits.
pub fn run(opening: Opening) -> ExitCode {
    let Some(mtm) = MainThreadMarker::new() else {
        eprintln!("grind-mac: AppKit runs on the main thread only");
        return ExitCode::FAILURE;
    };
    ignore_arguments_as_files();
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
    // Before anything asks for the shared controller, so that this one is it.
    let controller = Controller::install(mtm);
    let delegate = Delegate::new(mtm, opening, controller);
    app.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    app.setMainMenu(Some(&menu_bar(&app, mtm)));
    app.run();
    ExitCode::SUCCESS
}

/// Tell AppKit not to open the command line's arguments as documents.
///
/// Left on, `NSTreatUnknownArgumentsAsOpen` has AppKit open every argument it does not
/// recognise as a file — the path [`Delegate::launch`] has already opened — and a second
/// document with the same title comes up beside the first. A drive's keys then moved the
/// selection in that second window, the key one, while its transcript read the first (run
/// 37052072298). What the command line means is `args.rs`'s decision; a Finder open arrives as
/// an Apple Event and is not touched by this. Registered rather than set, so an explicit
/// `-NSTreatUnknownArgumentsAsOpen YES` still wins.
fn ignore_arguments_as_files() {
    let key = NSString::from_str("NSTreatUnknownArgumentsAsOpen");
    let no = NSString::from_str("NO");
    let values: [&AnyObject; 1] = [&no];
    let defaults = NSDictionary::from_slices(&[&*key], &values);
    // SAFETY: a dictionary of property-list values, which is what the registration domain holds.
    unsafe { NSUserDefaults::standardUserDefaults().registerDefaults(&defaults) };
}

/// The standard About panel, saying which build this is the way every other window's About does:
/// the crate's version, the commit and whether the tree was clean, and how and when it was built
/// (`grind_core::build_info`, the one place the stamp is formatted).
fn about(mtm: MainThreadMarker) {
    let version = env!("CARGO_PKG_VERSION");
    let stamp = grind_core::build_info::describe_version(version);
    // Everything after the first line: the commit and the date, as the panel's credits — and
    // then every third-party component and its licence (doc/third-party.md), since the credits
    // are the panel's one area that scrolls and the place a Mac application acknowledges them.
    let mut credits = stamp.lines().skip(1).collect::<Vec<_>>().join("\n");
    credits.push_str("\n\n");
    credits.push_str(&grind_core::third_party::notices());
    let build = format!(
        "{} {}",
        grind_core::build_info::COMMIT,
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        }
    );
    let credits = NSAttributedString::from_nsstring(&NSString::from_str(&credits));
    // SAFETY: AppKit's own option keys, each to the type its documentation names.
    unsafe {
        let keys: [&NSAboutPanelOptionKey; 3] = [
            NSAboutPanelOptionApplicationVersion,
            NSAboutPanelOptionVersion,
            NSAboutPanelOptionCredits,
        ];
        let application = NSString::from_str(version);
        let build = NSString::from_str(&build);
        let values: [&AnyObject; 3] = [&application, &build, &credits];
        let options = NSDictionary::from_slices(&keys, &values);
        NSApplication::sharedApplication(mtm).orderFrontStandardAboutPanelWithOptions(&options);
    }
}
