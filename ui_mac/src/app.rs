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
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject, Sel};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSAlert, NSApplication, NSApplicationActivationPolicy, NSApplicationDelegate,
    NSEventModifierFlags, NSMenu, NSMenuItem,
};
use objc2_foundation::{NSNotification, NSObject, NSObjectProtocol, NSString};

use crate::Opening;
use crate::document::{Controller, Document};
use crate::menu::{self, Action, COMMAND_SELECTOR, Command, Item, MENUS, Menu, Role};

/// What the delegate was launched to do, until it has done it.
pub struct Launch {
    opening: RefCell<Option<Opening>>,
    controller: Retained<Controller>,
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
        });
        // SAFETY: `init` is `NSObject`'s designated initialiser.
        unsafe { msg_send![super(this), init] }
    }

    /// The whole of what a command does — matched exhaustively, so a command with no handler
    /// fails the build.
    fn perform(&self, command: Command) {
        let kind = match command {
            Command::NewSheet => DocumentKind::Spreadsheet,
            Command::NewText => DocumentKind::Text,
            Command::GoTo => {
                // The front document's name box, when the front document is a spreadsheet.
                let pane = self
                    .ivars()
                    .controller
                    .currentDocument()
                    .and_then(|document| document.downcast::<Document>().ok())
                    .and_then(|document| document.pane());
                if let Some(pane) = pane {
                    pane.focus_name_box();
                }
                return;
            }
        };
        if let Err(message) = self.ivars().controller.new_document(kind) {
            alert(self.mtm(), "The document could not be made.", &message);
        }
    }

    fn launch(&self, opening: Opening) {
        let mtm = self.mtm();
        let controller = &self.ivars().controller;
        let opened = match (&opening.path, opening.kind) {
            (Some(path), _) => controller.open_path(path).map(Some),
            (None, Some(kind)) => controller.new_document(kind).map(|()| None),
            // The welcome window is M9's; until then an empty spreadsheet is the launch.
            (None, None) => controller
                .new_document(DocumentKind::Spreadsheet)
                .map(|()| None),
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

/// Say what went wrong, in a sheet-less alert — the one modal this milestone has.
fn alert(mtm: MainThreadMarker, message: &str, detail: &str) {
    let alert = NSAlert::new(mtm);
    alert.setMessageText(&NSString::from_str(message));
    alert.setInformativeText(&NSString::from_str(detail));
    alert.runModal();
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
