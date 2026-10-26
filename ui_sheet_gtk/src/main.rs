// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `grind-sheet-gtk` — the GNOME spreadsheet. A renderer and an event forwarder that owns nothing:
//! all state is in `grind-sheet`'s [`App`]; this crate holds a window, a grid drawing what
//! [`App::get_viewport`] returns, and the editing around it. A field here that is not presentation
//! means the core is missing something.
//!
//! `doc/sheet-shell.md`'s **"Four surfaces"** governs the chrome, and [`actions`] enforces it: one
//! table of verbs read by the wiring, the menus, the shortcuts window and the command palette, so
//! no feature can be added without becoming findable. `chrome_tests` is the check.
//!
//! **The core pushes; this never polls** (rule 3). The observer is a channel sender — it must be
//! `Send`, and a widget is not — drained by one local future that coalesces a burst of changes
//! into one refresh.

mod chart;
mod chart_dialog;
mod chrome;
mod code;
mod filter_ui;
mod find;
mod formatting;
mod formula_ux;
mod geom;
mod grid;
mod keymap;
mod licences;
mod lint;
mod palette;
mod rules;
mod settings;
mod state;
mod theme;
mod words;

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::rc::Rc;
use std::sync::Arc;

use libadwaita as adw;
use libadwaita::gtk;
use libadwaita::prelude::*;

use grind_sheet::{App, DocumentKind, Form, Observer, RecalcMode, a1, csv};
use gtk::{gio, glib};

use grid::{Grid, Notice};

/// The reverse-DNS identity GNOME keys a window, its icon and its settings on.
const APP_ID: &str = "io.github.fwilhe2.Sheet";

/// What this shell edits. The one input `save_name` needs beyond the form.
const KIND: DocumentKind = DocumentKind::Spreadsheet;

/// The word processor's window — what a text document opened here is handed to, the way
/// `grind-text-gtk` hands a spreadsheet to this one (`doc/suite.md`'s cross-app handoff).
const TEXT_APP: &str = "grind-text-gtk";

fn main() -> ExitCode {
    let app = Arc::new(App::new());

    // The document is opened before the toolkit starts, so a bad path is a shell error
    // with an exit code rather than an empty window with a dialog in front of it. Like
    // every other shell here, a missing file is an error and not a new document.
    let mut args = std::env::args_os().skip(1);
    let mut path: Option<PathBuf> = args.next().map(PathBuf::from);
    // `--render-to <png>` draws one frame, writes it and exits — not a user feature, but how a
    // machine checks a custom-drawn widget still draws.
    let mut render_to: Option<PathBuf> = None;
    // `--overlay names|roles|all` turns the overlays on for that frame, so the modes are
    // assertable too.
    let mut overlays = grind_sheet::view::Overlays::NONE;
    while let Some(flag) = args.next() {
        match flag.to_str() {
            Some("--render-to") => {
                render_to = Some(PathBuf::from(
                    args.next().unwrap_or_else(|| "grid.png".into()),
                ));
            }
            Some("--overlay") => {
                overlays = match args.next().as_deref().and_then(std::ffi::OsStr::to_str) {
                    Some("names") => grind_sheet::view::Overlays::NAMES,
                    Some("roles") => grind_sheet::view::Overlays::ROLES,
                    Some("all") => grind_sheet::view::Overlays::ALL,
                    other => {
                        eprintln!(
                            "grind-sheet-gtk: --overlay takes names, roles or all, not {}",
                            other.unwrap_or("nothing")
                        );
                        return ExitCode::FAILURE;
                    }
                };
            }
            _ => {}
        }
    }
    // A file on the command line is opened by the window once it exists (`Ui::load`): a large
    // workbook takes a minute to import, and a process that has not shown a window by then is one
    // the desktop offers to kill.
    let given = path.take();
    // A window with no file is a new document, and a new document here speaks the desktop's
    // language (`fresh_document`).
    if given.is_none() {
        let _ = app.open_bytes("untitled.fods", &fresh_document());
    }

    let application = adw::Application::builder().application_id(APP_ID).build();
    application.connect_activate(move |application| {
        theme::install();
        let ui = Ui::build(application, &app, path.clone());
        if overlays.any() {
            ui.grid.set_overlays(overlays);
        }
        ui.window.present();
        match given.clone() {
            Some(given) => {
                let render_to = render_to.clone();
                ui.load_then(&given, move |ui| {
                    if let Some(target) = render_to {
                        render_once(&ui.window, target);
                    }
                });
            }
            None => {
                if let Some(target) = render_to.clone() {
                    render_once(&ui.window, target);
                }
            }
        }
    });

    // The file argument was consumed above; GApplication must not try to parse it.
    match application.run_with_args::<&str>(&[]) {
        code if code == glib::ExitCode::SUCCESS => ExitCode::SUCCESS,
        _ => ExitCode::FAILURE,
    }
}

/// The window and the pieces that have to be told when something changes.
///
/// Everything here is presentation: which file this is, whether it has unsaved changes, and
/// the widgets. The document lives in `App`.
struct Ui {
    app: Arc<App>,
    window: adw::ApplicationWindow,
    grid: Grid,
    /// The two pages of the window: the grid, and its projection.
    stack: gtk::Stack,
    source: gtk::TextView,
    /// Raised while the code view is being filled, so placing its cursor does not read back as
    /// the reader moving it — `formatting::Strip`'s latch, for the same reason.
    updating: Cell<bool>,
    /// The code view line the selection is on, as last marked. The latch above is not enough: GTK
    /// does not always deliver `notify::cursor-position` inside the call that caused it, so the
    /// handler is made idempotent — the same line twice does nothing.
    marked: Cell<Option<usize>>,
    title: adw::WindowTitle,
    toasts: adw::ToastOverlay,
    banner: adw::Banner,
    /// The status bar's number spelling — `1.234,5 · de-DE` — refreshed with the document.
    spelling_badge: gtk::Button,
    /// Beside it, the spelling language: `🌐` for every language, a dictionary's flag for one,
    /// `Off` — a menu over the same `spell-language` action as View ▸ Spelling Language.
    spelling_picker: gtk::MenuButton,
    tabs: Rc<chrome::Tabs>,
    strip: Rc<formatting::Strip>,
    formula_bar: Rc<chrome::FormulaBar>,
    /// Ctrl+F and Ctrl+H (`find.rs`).
    find: Rc<find::Find>,
    undo: gtk::Button,
    redo: gtk::Button,
    path: RefCell<Option<PathBuf>>,
    /// The name an imported workbook goes by until it is saved — shown as the
    /// title and offered by Save As. `None` for every document that came from ODF.
    imported: RefCell<Option<String>>,
    dirty: Cell<bool>,
    /// Set by a load, and consumed by the change it is about to cause — opening a document
    /// notifies like any other change, and it must not leave the new one marked as modified.
    loading: Cell<bool>,
    /// A file is being read on a worker thread (`load_then`).
    opening: Cell<bool>,
    /// A recalculation is running on a worker thread (`recalc_in_background`).
    recalculating: Cell<bool>,
    /// The actions `set_busy` switched off, to switch back on.
    parked: RefCell<Vec<gio::SimpleAction>>,
    /// `editor`'s latch: a close that is waiting on a save must not ask again.
    closing: Cell<bool>,
    /// A text document somebody tried to open here, waiting on the banner's *Open in Text* —
    /// and while it is set, the banner's button means that rather than *Recalculate Anyway*.
    handoff: RefCell<Option<PathBuf>>,
    /// Whether spelling is checked, and in what — remembered across windows and sessions
    /// (`grind_spell::preference`, `doc/spelling.md` "In a spreadsheet"), so turning it off once
    /// keeps it off.
    spelling: Cell<grind_spell::Setting>,
    /// What the last setting that was not *Off* was, so Check Spelling turns back on to the
    /// language somebody chose rather than to Automatic.
    spelling_on: Cell<grind_spell::Setting>,
    /// What that setting came to for this document — `None` while nothing is checked.
    checked: Cell<Option<grind_spell::Choice>>,
    /// The guess has read all a guess reads and cannot change, so the document is no longer
    /// sampled on every edit. Cleared whenever a dictionary is attached afresh.
    settled: Cell<bool>,
}

impl Ui {
    fn build(application: &adw::Application, app: &Arc<App>, path: Option<PathBuf>) -> Rc<Self> {
        let grid = Grid::new(app.clone());
        let scroller = gtk::ScrolledWindow::builder()
            .hexpand(true)
            .vexpand(true)
            .child(&grid)
            .build();

        // The code view, on the other page of a stack. A stack rather than a split: a split is a
        // second viewport to keep in step, and one page at a time carries the correspondence.
        let source = code::build();
        let source_scroller = gtk::ScrolledWindow::builder()
            .hexpand(true)
            .vexpand(true)
            .child(&source)
            .build();
        let stack = gtk::Stack::new();
        stack.add_named(&scroller, Some("grid"));
        stack.add_named(&source_scroller, Some("source"));

        let toasts = adw::ToastOverlay::new();
        toasts.set_child(Some(&stack));

        let title = adw::WindowTitle::new(&document_name(path.as_deref()), "");
        let header = adw::HeaderBar::new();
        header.set_title_widget(Some(&title));

        let open = gtk::Button::from_icon_name("document-open-symbolic");
        open.set_tooltip_text(Some("Open"));
        open.set_action_name(Some("win.open"));
        header.pack_start(&open);

        let undo = gtk::Button::from_icon_name("edit-undo-symbolic");
        undo.set_action_name(Some("win.undo"));
        undo.set_tooltip_text(Some("Undo"));
        let redo = gtk::Button::from_icon_name("edit-redo-symbolic");
        redo.set_action_name(Some("win.redo"));
        redo.set_tooltip_text(Some("Redo"));
        let history = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        history.add_css_class("linked");
        history.append(&undo);
        history.append(&redo);
        header.pack_start(&history);

        // The end of the bar, packed right to left: the primary menu outermost, then the two
        // buttons that are about *looking* rather than about the document — which is the
        // HIG's split between the two ends of a header bar.
        let menu = gtk::MenuButton::builder()
            .icon_name("open-menu-symbolic")
            .menu_model(&primary_menu())
            .tooltip_text("Main Menu")
            .build();
        header.pack_end(&menu);
        header.pack_end(&chrome::view_menu(&grid));

        // The palette's own button. A header bar has no room to teach Ctrl+K, and a verb
        // nobody can find is a verb that does not exist — so the growth valve gets the one
        // affordance that says it is there (`doc/sheet-shell.md`, "Four surfaces").
        let find = gtk::Button::builder()
            .icon_name("system-search-symbolic")
            .action_name("win.palette")
            .tooltip_text("Find a command (Ctrl+K)")
            .build();
        header.pack_end(&find);

        let banner = adw::Banner::new("");
        let spelling_badge = chrome::number_badge();
        let spelling_picker = chrome::spelling_picker();
        let add_sheet = gtk::Button::from_icon_name("list-add-symbolic");
        add_sheet.set_tooltip_text(Some("Add Sheet"));
        add_sheet.set_action_name(Some("win.sheet-add"));
        add_sheet.add_css_class("flat");
        let tabs = chrome::Tabs::new(&grid, app, &add_sheet);
        let strip = formatting::strip(&grid, app);

        let view = adw::ToolbarView::builder().content(&toasts).build();
        view.add_top_bar(&header);
        // The window's only toolbar: a control belongs there when it reads and writes a property
        // of the selection (`doc/sheet-shell.md`, "Four surfaces"); verbs go to the context menu
        // and the palette.
        view.add_top_bar(&chrome::format_bar(&strip.widget));
        let formula_bar = chrome::formula_bar(&grid, app, true);
        view.add_top_bar(&formula_bar.widget);
        // Under the formula bar, so the bar that shows a found cell's formula is directly
        // above the one that found it. Hidden until Ctrl+F: it is not a fifth surface.
        let find = find::Find::new(app, &grid);
        view.add_top_bar(&find.bar);
        view.add_top_bar(&banner);
        view.add_bottom_bar(&chrome::bottom_bar(
            &tabs.widget,
            &chrome::status_bar(&grid, app, &spelling_badge, &spelling_picker),
        ));

        let window = adw::ApplicationWindow::builder()
            .application(application)
            .default_width(1100)
            .default_height(700)
            .content(&view)
            .build();
        // The grid is what the keys are for, so it starts focused rather than waiting for a
        // click to make the arrow keys work. Two traits spell `set_focus`; this is the one.
        gtk::prelude::GtkWindowExt::set_focus(&window, Some(&grid));

        let remembered = grind_spell::preference::load(grind_core::DocumentKind::Spreadsheet);
        let ui = Rc::new(Self {
            app: app.clone(),
            window,
            grid,
            stack,
            source,
            updating: Cell::new(false),
            marked: Cell::new(None),
            title,
            toasts,
            banner,
            spelling_badge,
            spelling_picker,
            tabs,
            strip,
            formula_bar,
            find,
            undo,
            redo,
            path: RefCell::new(path),
            dirty: Cell::new(false),
            loading: Cell::new(false),
            opening: Cell::new(false),
            recalculating: Cell::new(false),
            parked: RefCell::new(Vec::new()),
            imported: RefCell::new(None),
            closing: Cell::new(false),
            handoff: RefCell::new(None),
            spelling: Cell::new(remembered),
            spelling_on: Cell::new(match remembered {
                grind_spell::Setting::Off => grind_spell::Setting::Automatic,
                on => on,
            }),
            checked: Cell::new(None),
            settled: Cell::new(false),
        });
        ui.wire(application);
        ui.check_spelling(false);
        ui.refresh();
        // Nothing has moved yet, so the status bar and the formula bar would otherwise stay
        // empty until the first keystroke.
        ui.grid.set_selection(ui.grid.selection());
        ui
    }

    fn wire(self: &Rc<Self>, application: &adw::Application) {
        // The bridge. `Observer` is `Send + Sync` and a widget is neither, so what crosses
        // is a token; the local future does the reading, *after* the mutation that sent it
        // has released the lock.
        let (sender, receiver) = async_channel::unbounded::<()>();
        self.app.set_observer(Arc::new(Bridge(sender)));
        // An edit's ripple is a recalculation of the whole document, which for a big one is
        // minutes: it is owed rather than done inside the edit, and paid for here, off this
        // thread, with the window still drawn (`recalc_in_background`).
        self.app.defer_recalc(true);
        glib::spawn_future_local(glib::clone!(
            #[strong(rename_to = ui)]
            self,
            async move {
                while receiver.recv().await.is_ok() {
                    // Drain: N changes are one repaint, which is what keeps a recalculation
                    // of a thousand cells from queueing a thousand frames.
                    while receiver.try_recv().is_ok() {}
                    let opened = ui.loading.get();
                    ui.dirty.set(!ui.loading.replace(false));
                    ui.refresh();
                    ui.grid.invalidate();
                    // A document just opened is checked in its own language; one being edited
                    // may still be deciding what its language is.
                    match opened {
                        true => ui.check_spelling(false),
                        false => ui.reconsider_spelling(),
                    }
                    if ui.app.take_recalc_owed() {
                        ui.recalc_in_background(true, false);
                    }
                }
            }
        ));

        // A finished replace is announced with an Undo button, the window's own toast for
        // an edit that touched more than the cell in front of somebody.
        let weak = Rc::downgrade(self);
        self.find.connect_said(move |text| {
            if let Some(ui) = weak.upgrade() {
                ui.undoable_toast(text);
            }
        });

        self.grid.connect_notice(glib::clone!(
            #[strong(rename_to = ui)]
            self,
            move |notice| ui.notice(notice)
        ));

        self.banner.connect_button_clicked(glib::clone!(
            #[strong(rename_to = ui)]
            self,
            move |_| match ui.handoff.borrow().is_some() {
                true => ui.hand_over(),
                false => ui.recalculate(),
            }
        ));

        for verb in actions() {
            let action = gio::SimpleAction::new(verb.name, None);
            let run = verb.run;
            action.connect_activate(glib::clone!(
                #[strong(rename_to = ui)]
                self,
                move |_, _| run(&ui)
            ));
            self.window.add_action(&action);
            if !verb.accels.is_empty() {
                application.set_accels_for_action(&format!("win.{}", verb.name), verb.accels);
            }
        }

        // Friendly formulas are a *view*, so the action carries the state a checkbox in the
        // menu reads — the one action here that is not a verb.
        let friendly = gio::SimpleAction::new_stateful(
            "friendly-formulas",
            None,
            &self.formula_bar.friendly().to_variant(),
        );
        friendly.connect_activate(glib::clone!(
            #[strong(rename_to = ui)]
            self,
            move |action, _| {
                let on = !ui.formula_bar.friendly();
                ui.formula_bar.set_friendly(on);
                action.set_state(&on.to_variant());
            }
        ));
        self.window.add_action(&friendly);

        // The two overlays, stateful: a way of looking at the document, so the toggle carries its
        // own state, and neither writes a byte — no confirmation, no dirty flag, no undo entry.
        for (name, names, roles) in [("show-names", true, false), ("show-roles", false, true)] {
            let on = match names {
                true => self.grid.overlays().names,
                false => self.grid.overlays().roles,
            };
            let action = gio::SimpleAction::new_stateful(name, None, &on.to_variant());
            action.connect_activate(glib::clone!(
                #[strong(rename_to = ui)]
                self,
                move |action, _| {
                    let mut overlays = ui.grid.overlays();
                    let on = match (names, roles) {
                        (true, _) => {
                            overlays.names = !overlays.names;
                            overlays.names
                        }
                        _ => {
                            overlays.roles = !overlays.roles;
                            overlays.roles
                        }
                    };
                    ui.grid.set_overlays(overlays);
                    action.set_state(&on.to_variant());
                    // The formula bar reads the name overlay too (§3.3), and it refreshes
                    // on a selection change rather than on a repaint — so tell it the
                    // selection it already has.
                    ui.grid.set_selection(ui.grid.selection());
                }
            ));
            self.window.add_action(&action);
            accelerate(application, name);
        }

        // The third reading: each formula shown instead of its result, as `grind sheet view
        // --formulas` prints it. Stateful and written to nothing, like the two above.
        let shown = gio::SimpleAction::new_stateful(
            "show-formulas",
            None,
            &self.grid.formulas().to_variant(),
        );
        shown.connect_activate(glib::clone!(
            #[strong(rename_to = ui)]
            self,
            move |action, _| {
                let on = !ui.grid.formulas();
                ui.grid.set_formulas(on);
                action.set_state(&on.to_variant());
            }
        ));
        self.window.add_action(&shown);

        // Spelling: one toggle, Shift+F7 as in every office suite, and a language choice beside
        // it. Both write the person's remembered setting, not the document — no undo entry, no
        // dirty flag — and apply to every spreadsheet window from then on, so *off* stays off.
        let check = gio::SimpleAction::new_stateful(
            "spell-check",
            None,
            &(self.spelling.get() != grind_spell::Setting::Off).to_variant(),
        );
        check.connect_activate(glib::clone!(
            #[strong(rename_to = ui)]
            self,
            move |_, _| {
                let next = match ui.spelling.get() {
                    grind_spell::Setting::Off => ui.spelling_on.get(),
                    _ => grind_spell::Setting::Off,
                };
                ui.set_spelling(next);
            }
        ));
        self.window.add_action(&check);
        accelerate(application, "spell-check");
        let language = gio::SimpleAction::new_stateful(
            TARGETED[0],
            Some(glib::VariantTy::STRING),
            &self.spelling.get().tag().to_variant(),
        );
        language.connect_activate(glib::clone!(
            #[strong(rename_to = ui)]
            self,
            move |_, target| {
                if let Some(setting) = target
                    .and_then(|v| v.get::<String>())
                    .and_then(|tag| grind_spell::Setting::from_tag(&tag))
                {
                    ui.set_spelling(setting);
                }
            }
        ));
        self.window.add_action(&language);

        // The document as its projection, on the other page of the stack — stateful like the
        // overlays: a reading, turned off by the same item.
        let source = gio::SimpleAction::new_stateful("show-source", None, &false.to_variant());
        source.connect_activate(glib::clone!(
            #[strong(rename_to = ui)]
            self,
            move |action, _| {
                let on = ui.stack.visible_child_name().as_deref() != Some("source");
                ui.show_source(on);
                action.set_state(&on.to_variant());
            }
        ));
        self.window.add_action(&source);
        accelerate(application, "show-source");

        // Moving the cursor in the source selects the cell that line projects — §6.2's map in
        // the direction that has to be built rather than assumed. `notify::cursor-position`
        // rather than a key handler, so a click, a drag and an arrow all reach it.
        self.source
            .buffer()
            .connect_cursor_position_notify(glib::clone!(
                #[strong(rename_to = ui)]
                self,
                move |_| ui.source_moved()
            ));

        self.window.connect_close_request(glib::clone!(
            #[strong(rename_to = ui)]
            self,
            move |_| ui.confirm_close()
        ));
    }

    // --- the code view ---

    /// Switch between the grid and its source.
    fn show_source(self: &Rc<Self>, on: bool) {
        self.stack.set_visible_child_name(match on {
            true => "source",
            false => "grid",
        });
        match on {
            true => {
                self.fill_source();
                let _ = self.source.grab_focus();
            }
            false => {
                gtk::prelude::GtkWindowExt::set_focus(&self.window, Some(&self.grid));
            }
        }
    }

    /// Paint the projection, with the active cell's own line marked.
    fn fill_source(self: &Rc<Self>) {
        let projection = self.app.project();
        let line = self
            .app
            .sheet_name(self.grid.sheet())
            .ok()
            .map(|name| a1::format(Some(&name), self.grid.selection().active))
            .and_then(|address| projection.line_of(&address));
        self.updating.set(true);
        code::fill(&self.source, &projection, line);
        self.marked.set(line);
        self.updating.set(false);
    }

    /// The source's cursor moved: select the cell that line projects. A **sheet's own name** is
    /// checked first: a bare name is also a cell address (`Sheet1` is column `SHEET`, row 1), and
    /// `sheet Sales {` should open the sheet.
    fn source_moved(self: &Rc<Self>) {
        if self.updating.get() || self.stack.visible_child_name().as_deref() != Some("source") {
            return;
        }
        let line = code::line_at_cursor(&self.source);
        // Already answered. Without this the window hangs: `set_selection` below leads to a
        // repaint, GTK delivers the cursor notify again afterwards, and the handler answers a
        // move that never happened.
        if self.marked.get() == Some(line) {
            return;
        }
        self.marked.set(Some(line));
        let projection = self.app.project();
        let Some(address) = projection.address_on_line(line) else {
            return;
        };
        if let Ok(sheet) = a1::sheet(&self.app, address) {
            self.grid.set_sheet(sheet);
        } else if let Ok((sheet, start, _end)) =
            a1::parse(address).and_then(|reference| a1::resolve(&self.app, &reference))
        {
            self.grid.set_sheet(sheet);
            self.grid.set_selection(keymap::Selection::at(start));
        }
        // The tag only — moving the cursor from the handler that runs because the cursor moved
        // is the loop this whole guard exists for. See `code::mark`.
        code::mark(&self.source, line);
    }

    /// Everything derived from the document, in one place, run after every change.
    fn refresh(self: &Rc<Self>) {
        let title = match (&*self.path.borrow(), &*self.imported.borrow()) {
            (None, Some(name)) => name.clone(),
            (path, _) => document_name(path.as_deref()),
        };
        self.title.set_title(&title);
        self.title.set_subtitle(match self.dirty.get() {
            true => "Unsaved changes",
            false => "",
        });
        self.undo.set_sensitive(self.app.can_undo());
        self.redo.set_sensitive(self.app.can_redo());
        self.tabs.refresh();
        self.spelling_badge
            .set_label(&grind_sheet::locale::badge(self.app.locale().as_ref()));
        // Undo, redo and a load all change the cell under an unmoved selection, which is
        // what the format strip is showing.
        self.strip.refresh();
    }

    fn toast(&self, text: &str) {
        self.toasts.add_toast(adw::Toast::new(text));
    }

    // --- spelling (`doc/spelling.md`, "In a spreadsheet") ---

    /// Attach a dictionary for what the Spelling setting says — on every open, and when the
    /// setting changes — and hand the grid the speller its cell menu teaches words to. `say`
    /// announces the outcome, which a change of setting wants and an open does not; a document
    /// in a language there is no dictionary for always says so, once, rather than underlining
    /// every word.
    fn check_spelling(self: &Rc<Self>, say: bool) {
        self.settled.set(false);
        let setting = self.spelling.get();
        self.spelling_picker.set_label(setting.chip(true));
        self.spelling_picker.set_tooltip_text(Some(&format!(
            "Spelling: {} — click to choose",
            setting.label()
        )));
        let outcome = setting.apply(&self.app);
        let speller = match outcome {
            Ok(Some((choice, speller))) => {
                self.checked.set(Some(choice));
                if say {
                    self.toast(&format!("Checking spelling in {}", choice.describe()));
                }
                Some(speller)
            }
            Ok(None) => {
                self.checked.set(None);
                if say {
                    self.toast("Spelling is off in every spreadsheet until it is turned back on");
                }
                None
            }
            Err(why) => {
                self.checked.set(None);
                self.toast(&format!("Spelling is not checked: {why}"));
                None
            }
        };
        self.grid.set_speller(speller);
        if let Some(action) = self.window.lookup_action("spell-check") {
            action.change_state(&self.checked.get().is_some().to_variant());
        }
    }

    /// Choose a spelling setting, remember it for every spreadsheet window, and apply it.
    fn set_spelling(self: &Rc<Self>, setting: grind_spell::Setting) {
        self.spelling.set(setting);
        if setting != grind_spell::Setting::Off {
            self.spelling_on.set(setting);
        }
        if let Err(why) =
            grind_spell::preference::save(grind_core::DocumentKind::Spreadsheet, setting)
        {
            self.toast(&format!("The setting was not remembered: {why}"));
        }
        if let Some(action) = self.window.lookup_action("spell-language") {
            action.change_state(&setting.tag().to_variant());
        }
        self.check_spelling(true);
    }

    /// Spelling Language…: the same choice the View menu's submenu makes, as a dialog the
    /// command palette can open — a palette row cannot carry the submenu's radio targets, and
    /// the submenu sits two levels down in a popover, which is where nobody looks first.
    fn spelling_language_dialog(self: &Rc<Self>) {
        let settings = grind_spell::Setting::ALL;
        let labels: Vec<&str> = settings.iter().map(|s| s.label()).collect();
        let choices = gtk::DropDown::from_strings(&labels);
        let current = self.spelling.get();
        choices.set_selected(settings.iter().position(|s| *s == current).unwrap_or(0) as u32);
        let dialog = adw::AlertDialog::new(
            Some("Spelling Language"),
            Some("Which dictionary checks spreadsheets. Remembered for every one."),
        );
        dialog.set_extra_child(Some(&choices));
        dialog.add_response("cancel", "Cancel");
        dialog.add_response("choose", "Choose");
        dialog.set_response_appearance("choose", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("choose"));
        dialog.set_close_response("cancel");
        dialog.choose(
            &self.window,
            gio::Cancellable::NONE,
            glib::clone!(
                #[strong(rename_to = ui)]
                self,
                move |response| {
                    if response != "choose" {
                        return;
                    }
                    if let Some(setting) = settings.get(choices.selected() as usize) {
                        ui.set_spelling(*setting);
                    }
                }
            ),
        );
    }

    /// While the language is only a guess — a new document, or one stating none — guess again
    /// as it fills, so the first words typed into an empty sheet decide it rather than the
    /// desktop's language (`grind_spell::reguess`). Settled once the document has more words
    /// than a guess reads, so a large sheet is not sampled on every edit.
    fn reconsider_spelling(self: &Rc<Self>) {
        use grind_spell::Source;
        let Some(choice) = self.checked.get() else {
            return;
        };
        if self.settled.get()
            || self.spelling.get() != grind_spell::Setting::Automatic
            || !matches!(choice.source, Source::Guessed | Source::Default)
        {
            return;
        }
        let words = grind_core::spell::Spelled::sample(&*self.app).len();
        if grind_spell::reguess(&self.app, self.spelling.get(), choice, words) {
            self.check_spelling(false);
        } else if words >= grind_core::spell::GUESS_SAMPLE {
            // A guess cannot change past the sample, so stop asking.
            self.settled.set(true);
        }
    }

    /// A toast with an Undo button — the HIG pattern for anything destructive that is
    /// cheaper to do and take back than to confirm first.
    fn undoable_toast(self: &Rc<Self>, text: &str) {
        let toast = adw::Toast::builder()
            .title(text)
            .button_label("Undo")
            .build();
        toast.connect_button_clicked(glib::clone!(
            #[strong(rename_to = ui)]
            self,
            move |_| {
                ui.app.undo();
            }
        ));
        self.toasts.add_toast(toast);
    }

    fn notice(self: &Rc<Self>, notice: Notice) {
        match notice {
            Notice::BadFormula(message, _) => self.toast(&format!("Not a formula: {message}")),
            Notice::Refused(message) => self.toast(&message),
            // A toast with Undo: the entry landed, and taking it back is the other answer.
            Notice::Read(sentence) => self.undoable_toast(&sentence),
            // A banner rather than a toast: this is a *state* the document is in, not an
            // event that happened, and it stays true until something recalculates.
            Notice::RecalcSkipped(spoiled) => {
                self.handoff.borrow_mut().take();
                self.banner.set_title(&match spoiled {
                    1 => "1 formula uses a function this build does not have — \
                          recalculating would replace its saved value"
                        .to_owned(),
                    n => format!(
                        "{n} formulas use functions this build does not have — \
                         recalculating would replace their saved values"
                    ),
                });
                self.banner.set_button_label(Some("Recalculate Anyway"));
                self.banner.set_revealed(true);
            }
            Notice::EditChart(index) => self.chart_dialog(Some(index)),
            Notice::DeleteChart(index) => self.delete_chart(index),
        }
    }

    fn recalculate(self: &Rc<Self>) {
        self.banner.set_revealed(false);
        self.recalc_in_background(false, true);
    }

    /// Recalculate the whole document on a worker thread (`App::recalc_in_place`), with a toast
    /// that says how far it has got. `merge` folds the result into the undo entry of the edit
    /// that owed it; `force` writes it even where it would turn a saved value into an error,
    /// which is what asking for a recalculation (F9, the banner) means and an edit's ripple
    /// does not.
    ///
    /// The window stays drawn, because the worker holds only a read lock; it is `set_busy` that
    /// keeps it from being written to, since a writer waiting behind that lock would stop every
    /// repaint behind it.
    fn recalc_in_background(self: &Rc<Self>, merge: bool, force: bool) {
        if self.recalculating.replace(true) {
            return;
        }
        let notice = adw::Toast::builder()
            .title("Recalculating…")
            .timeout(0)
            .build();
        self.toasts.add_toast(notice.clone());
        self.set_busy(true);

        let progress = Arc::new((
            std::sync::atomic::AtomicUsize::new(0),
            std::sync::atomic::AtomicUsize::new(0),
        ));
        let finished = Rc::new(Cell::new(false));
        glib::timeout_add_local(
            std::time::Duration::from_millis(150),
            glib::clone!(
                #[strong]
                notice,
                #[strong]
                progress,
                #[strong]
                finished,
                move || {
                    if finished.get() {
                        return glib::ControlFlow::Break;
                    }
                    let done = progress.0.load(std::sync::atomic::Ordering::Relaxed);
                    let total = progress.1.load(std::sync::atomic::Ordering::Relaxed);
                    if let Some(percent) = (done * 100).checked_div(total) {
                        notice.set_title(&format!("Recalculating… {percent}%"));
                    }
                    glib::ControlFlow::Continue
                }
            ),
        );

        let ui = self.clone();
        glib::spawn_future_local(async move {
            let app = ui.app.clone();
            let result = gio::spawn_blocking(move || {
                app.recalc_in_place(merge, force, |done, total| {
                    progress.0.store(done, std::sync::atomic::Ordering::Relaxed);
                    progress
                        .1
                        .store(total, std::sync::atomic::Ordering::Relaxed);
                })
            })
            .await;
            finished.set(true);
            notice.dismiss();
            ui.set_busy(false);
            ui.recalculating.set(false);
            match result {
                // The document changed while it was being computed: ask again, on the new one.
                Ok(Ok(None)) => ui.recalc_in_background(false, force),
                Ok(Ok(Some(recalc))) if !force => {
                    if recalc.spoiled > 0 {
                        ui.grid.report(Notice::RecalcSkipped(recalc.spoiled));
                    }
                }
                Ok(Ok(Some(recalc))) if recalc.spoiled > 0 => ui.undoable_toast(&counted(
                    recalc.spoiled,
                    "cell became an error",
                    "cells became errors",
                )),
                Ok(Ok(Some(recalc))) if recalc.changed > 0 => ui.toast(&counted(
                    recalc.changed,
                    "cell recalculated",
                    "cells recalculated",
                )),
                Ok(Ok(Some(_))) => ui.toast("Already up to date"),
                Ok(Err(error)) => ui.toast(&error.to_string()),
                Err(_) => ui.toast("Recalculation failed"),
            }
            ui.refresh();
            ui.grid.invalidate();
        });
    }

    /// Shut the window against edits while a worker thread has the document: the page, the bars
    /// and every action — which takes the menus, the buttons and the accelerators with it. The
    /// close button stays, and so do scrolling and the tabs of the grid it is showing.
    fn set_busy(&self, busy: bool) {
        self.stack.set_sensitive(!busy);
        self.strip.widget.set_sensitive(!busy);
        self.formula_bar.widget.set_sensitive(!busy);
        self.find.bar.set_sensitive(!busy);
        if busy {
            self.undo.set_sensitive(false);
            self.redo.set_sensitive(false);
            let mut parked = self.parked.borrow_mut();
            for name in self.window.list_actions() {
                if let Some(action) = self
                    .window
                    .lookup_action(&name)
                    .and_downcast::<gio::SimpleAction>()
                    && action.is_enabled()
                {
                    action.set_enabled(false);
                    parked.push(action);
                }
            }
        } else {
            for action in self.parked.borrow_mut().drain(..) {
                action.set_enabled(true);
            }
        }
    }

    // --- files ---

    fn save(self: &Rc<Self>) {
        let path = self.path.borrow().clone();
        match path {
            Some(path) => self.write(&path),
            None => self.save_as(),
        }
    }

    fn write(self: &Rc<Self>, path: &Path) {
        match self.app.save_file(path) {
            Ok(()) => {
                *self.path.borrow_mut() = Some(path.to_owned());
                *self.imported.borrow_mut() = None;
                self.dirty.set(false);
                // No "Saved" toast: the subtitle's "Unsaved changes" clearing is the
                // confirmation, and routine success asking to be noticed is noise. A save
                // that *fails* still says so below.
                self.refresh();
                remember_recent(path);
                if self.closing.get() {
                    self.window.close();
                }
            }
            Err(error) => {
                // A failed save cancels the close, or the work is gone.
                self.closing.set(false);
                self.toast(&format!("Could not save: {error}"));
            }
        }
    }

    fn save_as(self: &Rc<Self>) {
        let dialog = gtk::FileDialog::builder()
            .title("Save As")
            .filters(&spreadsheet_save_filters())
            .initial_name(
                self.imported
                    .borrow()
                    .clone()
                    .unwrap_or_else(|| save_name(self.path.borrow().as_deref())),
            )
            .build();
        dialog.save(
            Some(&self.window),
            gio::Cancellable::NONE,
            glib::clone!(
                #[strong(rename_to = ui)]
                self,
                move |result| match result.ok().and_then(|file| file.path()) {
                    Some(path) => ui.write(&path),
                    // Cancelled: whatever was waiting on the save is off too.
                    None => ui.closing.set(false),
                }
            ),
        );
    }

    fn open(self: &Rc<Self>) {
        self.confirm_discard(glib::clone!(
            #[strong(rename_to = ui)]
            self,
            move || {
                let dialog = gtk::FileDialog::builder()
                    .title("Open")
                    .filters(&spreadsheet_filters())
                    .build();
                dialog.open(
                    Some(&ui.window),
                    gio::Cancellable::NONE,
                    glib::clone!(
                        #[strong]
                        ui,
                        move |result| {
                            if let Some(path) = result.ok().and_then(|file| file.path()) {
                                ui.load(&path);
                            }
                        }
                    ),
                );
            }
        ));
    }

    fn load(self: &Rc<Self>, path: &Path) {
        self.load_then(path, |_| {});
    }

    /// Open `path`, then call `done`. Everything slow — reading, importing, parsing — runs on a
    /// worker thread (which touches no `App` and no widget), so the window stays
    /// drawn and answers the desktop's pings however large the file is; only handing the
    /// finished document over happens here.
    fn load_then(self: &Rc<Self>, path: &Path, done: impl FnOnce(&Rc<Self>) + 'static) {
        if self.opening.replace(true) {
            return self.toast("Still opening another file");
        }
        let notice = adw::Toast::builder()
            .title(format!(
                "Opening {} — a large file takes a while…",
                path.file_name().unwrap_or_default().to_string_lossy()
            ))
            .timeout(0)
            .build();
        // Said only when the open is still going after a moment, or every small file flashes it.
        let pending = Rc::new(Cell::new(true));
        {
            let (ui, notice, pending) = (self.clone(), notice.clone(), pending.clone());
            glib::timeout_add_local_once(std::time::Duration::from_millis(400), move || {
                if pending.get() {
                    ui.toasts.add_toast(notice);
                }
            });
        }
        self.set_busy(true);
        let ui = self.clone();
        let path = path.to_owned();
        glib::spawn_future_local(async move {
            let worker_path = path.clone();
            let prepared = gio::spawn_blocking(move || -> Result<_, String> {
                let bytes = std::fs::read(&worker_path).map_err(|e| e.to_string())?;
                // Cross-app handoff: the ODS reader is tolerant by construction and would hand
                // back a document with no sheets in it, so the kind is asked first.
                if is_text(&worker_path, &bytes) {
                    return Ok(None);
                }
                // Named after the file alone, which is what the title shows and Save As offers.
                let name = worker_path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy();
                let imported =
                    grind_open::import(&name, &bytes, &grind_open::nowhere).transpose()?;
                let document = match &imported {
                    Some(imported) => grind_sheet::read_bytes(&imported.name, &imported.odf),
                    None => grind_sheet::read_bytes(&worker_path.display().to_string(), &bytes),
                }
                .map_err(|e| e.to_string())?;
                Ok(Some((document, imported)))
            })
            .await;
            pending.set(false);
            notice.dismiss();
            ui.set_busy(false);
            ui.opening.set(false);
            match prepared {
                Ok(Ok(None)) => ui.offer_handoff(&path),
                Ok(Ok(Some((document, imported)))) => {
                    // The open's own notification is swallowed so an ODF document comes up
                    // unmodified. An imported workbook or CSV is *meant* to come up modified —
                    // nothing has saved it — so for one the notification is let through.
                    ui.loading.set(imported.is_none());
                    match ui.app.open_document(document) {
                        Ok(()) => {
                            // An import has never been saved: Save asks where the ODF goes.
                            *ui.path.borrow_mut() = imported.is_none().then(|| path.clone());
                            *ui.imported.borrow_mut() = None;
                            ui.grid.set_sheet(0);
                            match imported {
                                Some(imported) => ui.adopt_import(imported),
                                None => remember_recent(&path),
                            }
                        }
                        Err(error) => {
                            ui.loading.set(false);
                            ui.toast(&format!("Could not open: {error}"));
                        }
                    }
                }
                Ok(Err(error)) => ui.toast(&format!("Could not open: {error}")),
                Err(_) => ui.toast("Could not open: the file was too much for the reader"),
            }
            done(&ui);
        });
    }

    /// The banner that says "this is a text document", with the button that opens it in the
    /// word processor — `grind-text-gtk`'s own banner, the other way round.
    fn offer_handoff(self: &Rc<Self>, path: &Path) {
        *self.handoff.borrow_mut() = Some(path.to_owned());
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        self.banner
            .set_title(&format!("{name} is a text document."));
        self.banner.set_button_label(Some("Open in Text"));
        self.banner.set_revealed(true);
    }

    fn hand_over(self: &Rc<Self>) {
        let Some(path) = self.handoff.borrow_mut().take() else {
            return;
        };
        self.banner.set_revealed(false);
        if std::process::Command::new(TEXT_APP)
            .arg(&path)
            .spawn()
            .is_err()
        {
            // Saying plainly that the sibling is not installed — a button that silently does
            // nothing is the alternative.
            self.toast(&format!("{TEXT_APP} is not installed"));
        }
    }

    /// An imported workbook: named after it, unsaved, and the report's sentence where it
    /// cannot be missed. Not added to the recent files — it is not a file this window can
    /// open *as*, and reopening it would import it again rather than open what was saved.
    fn adopt_import(self: &Rc<Self>, imported: grind_open::Imported) {
        *self.imported.borrow_mut() = Some(imported.name);
        self.dirty.set(true);
        self.refresh();
        let toast = adw::Toast::builder()
            .title(&imported.summary)
            .timeout(10)
            .build();
        self.toasts.add_toast(toast);
    }

    // --- CSV, the one non-ODF format (`doc/not-doing.md` §2) ---

    /// *Import CSV…* — a delimited file read in **at the cursor**, one undo step. A picker and
    /// nothing else: the delimiter comes from the file's content and the destination from the
    /// selection, so there is no dialog to get wrong. The other knobs are `grind sheet
    /// import-csv`'s; `csv::Import::sniffed` is which a window sets.
    fn import_csv(self: &Rc<Self>) {
        self.pick_csv(String::new());
    }

    /// *Import CSV with Options…* — the options asked first, in words (`csv::Import::amended`:
    /// `delimiter=semicolon locale=de-DE text formulas trim no-dates`), then the same picker.
    fn import_csv_with(self: &Rc<Self>) {
        let entry = gtk::Entry::builder()
            .placeholder_text("delimiter=semicolon locale=de-DE text formulas trim no-dates")
            .activates_default(true)
            .build();
        let dialog = adw::AlertDialog::new(
            Some("Import CSV with Options"),
            Some("Leave empty to let the file decide the delimiter."),
        );
        dialog.set_extra_child(Some(&entry));
        dialog.add_response("cancel", "Cancel");
        dialog.add_response("choose", "Choose File…");
        dialog.set_response_appearance("choose", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("choose"));
        dialog.set_close_response("cancel");
        dialog.choose(
            &self.window,
            gio::Cancellable::NONE,
            glib::clone!(
                #[strong(rename_to = ui)]
                self,
                move |response| {
                    if response == "choose" {
                        ui.pick_csv(entry.text().to_string());
                    }
                }
            ),
        );
    }

    fn pick_csv(self: &Rc<Self>, words: String) {
        let dialog = gtk::FileDialog::builder()
            .title("Import CSV")
            .filters(&csv_filters())
            .build();
        dialog.open(
            Some(&self.window),
            gio::Cancellable::NONE,
            glib::clone!(
                #[strong(rename_to = ui)]
                self,
                move |result| {
                    if let Some(path) = result.ok().and_then(|file| file.path()) {
                        ui.read_csv(&path, &words);
                    }
                }
            ),
        );
    }

    fn read_csv(self: &Rc<Self>, path: &Path, words: &str) {
        let text = match std::fs::read(path)
            .map_err(|error| error.to_string())
            .and_then(|bytes| csv::decode(bytes).map_err(str::to_owned))
        {
            Ok(text) => text,
            // Both failures name the file, because the picker has closed by now and the
            // sentence is all that is left of it.
            Err(why) => return self.toast(&format!("{}: {why}", document_name(Some(path)))),
        };
        let sheet = self.grid.sheet();
        let at = self.grid.selection().active;
        let options = match csv::Import::sniffed(&text).amended(words) {
            Ok(options) => options,
            Err(why) => return self.toast(&why),
        };
        match self
            .app
            .import_csv(sheet, at, &text, &options, RecalcMode::Document)
        {
            Ok(outcome) => {
                if let Some(recalc) = outcome.recalc.filter(|recalc| recalc.spoiled > 0) {
                    self.grid.report(Notice::RecalcSkipped(recalc.spoiled));
                }
                self.toast(&format!(
                    "{} imported at {}",
                    counted(outcome.cells, "cell", "cells"),
                    a1::format(None, at)
                ));
            }
            Err(error) => self.toast(&error.to_string()),
        }
    }

    /// *Export CSV…* — the selection, or everything the sheet uses when one cell is selected, as
    /// each cell **shows**. The name picks the dialect: `.tsv` writes tabs
    /// (`csv::Dialect::for_name`).
    fn export_csv(self: &Rc<Self>) {
        let suggested = match self.path.borrow().as_deref().and_then(Path::file_stem) {
            Some(stem) => format!("{}.csv", stem.to_string_lossy()),
            None => "Untitled.csv".to_owned(),
        };
        let dialog = gtk::FileDialog::builder()
            .title("Export CSV")
            .filters(&csv_save_filters())
            .initial_name(suggested)
            .build();
        dialog.save(
            Some(&self.window),
            gio::Cancellable::NONE,
            glib::clone!(
                #[strong(rename_to = ui)]
                self,
                move |result| {
                    if let Some(path) = result.ok().and_then(|file| file.path()) {
                        ui.write_csv(&path);
                    }
                }
            ),
        );
    }

    fn write_csv(self: &Rc<Self>, path: &Path) {
        let sheet = self.grid.sheet();
        let (mut start, mut end) = self.grid.selection().rect();
        // One cell is a cursor, not a range — the same reading `Format as Table` gives it.
        if start == end {
            let Ok((rows, cols)) = self.app.used_extent(sheet) else {
                return self.toast("No such sheet");
            };
            if rows == 0 || cols == 0 {
                return self.toast("There is nothing in this sheet to export");
            }
            (start, end) = (
                grind_sheet::Pos::new(0, 0),
                grind_sheet::Pos::new(rows - 1, cols - 1),
            );
        }
        let name = document_name(Some(path));
        let options = csv::Export {
            dialect: csv::Dialect::for_name(&name),
            ..csv::Export::default()
        };
        match self
            .app
            .export_csv(sheet, start, end, &options)
            .map_err(|error| error.to_string())
            .and_then(|text| {
                grind_core::atomic::write(path, text).map_err(|error| error.to_string())
            }) {
            Ok(()) => self.toast(&format!(
                "{}:{} exported to {name}",
                a1::format(None, start),
                a1::format(None, end)
            )),
            Err(why) => self.toast(&format!("Could not export: {why}")),
        }
    }

    fn new_document(self: &Rc<Self>) {
        self.confirm_discard(glib::clone!(
            #[strong(rename_to = ui)]
            self,
            move || {
                // There is no `App::reset`, and there does not need to be: an empty document
                // written and read back is one, through the same path a file takes.
                let bytes = fresh_document();
                ui.loading.set(true);
                if ui.app.open_bytes("untitled.fods", &bytes).is_ok() {
                    *ui.path.borrow_mut() = None;
                    ui.grid.set_sheet(0);
                }
            }
        ));
    }

    // --- names ---

    /// The names dialog: every defined name, editable in place, and a row to add one. Each row is
    /// an `adw::EntryRow` over the definition, so editing is the same gesture as reading; the core
    /// does every refusal, and its message is the toast.
    fn manage_names(self: &Rc<Self>) {
        let list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .build();
        list.add_css_class("boxed-list");
        let empty = gtk::Label::builder()
            .label("No names yet. Name a range to use it in a formula: SUM(expenses).")
            .wrap(true)
            .xalign(0.0)
            .build();
        empty.add_css_class("dim-label");

        let toasts = adw::ToastOverlay::new();
        // The window's own toast overlay is *under* the dialog, so a refusal shown there
        // would be invisible exactly when it matters.
        let say: Rc<dyn Fn(&str)> = {
            let toasts = toasts.clone();
            Rc::new(move |text: &str| toasts.add_toast(adw::Toast::new(text)))
        };

        // Adding: a name and a definition, prefilled with the selection, because naming what
        // is selected is why the dialog was opened nine times in ten.
        let (start, end) = self.grid.selection().rect();
        let selected = match start == end {
            true => a1::format(None, start),
            false => format!("{}:{}", a1::format(None, start), a1::format(None, end)),
        };
        let new_name = adw::EntryRow::builder().title("Name").build();
        let new_definition = adw::EntryRow::builder()
            .title("Definition")
            .text(&selected)
            .build();
        let add_list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .build();
        add_list.add_css_class("boxed-list");
        add_list.append(&new_name);
        add_list.append(&new_definition);
        let add = gtk::Button::with_label("Add");
        add.add_css_class("suggested-action");
        add.set_halign(gtk::Align::End);

        // Whether the list has anything in it is the only thing two places both need to
        // know, and a row deletes itself, so this is all the shared state there is.
        let sync_empty: Rc<dyn Fn()> = {
            let (list, empty, app) = (list.clone(), empty.clone(), self.app.clone());
            Rc::new(move || {
                let any = !app.names().is_empty();
                list.set_visible(any);
                empty.set_visible(!any);
            })
        };
        for (name, expression) in self.app.names() {
            list.append(&name_row(&self.app, &name, &expression, &say, &sync_empty));
        }
        sync_empty();

        add.connect_clicked(glib::clone!(
            #[strong(rename_to = app)]
            self.app,
            #[weak]
            list,
            #[strong]
            new_name,
            #[strong]
            new_definition,
            #[strong]
            say,
            #[strong]
            sync_empty,
            move |_| {
                let name = new_name.text();
                match define(&app, &name, &new_definition.text()) {
                    Ok(expression) => {
                        // Redefining an existing name edits its row rather than adding a
                        // second one with the same title.
                        match row_named(&list, &name) {
                            Some(row) => row.set_text(&definition_text(&expression)),
                            None => {
                                list.append(&name_row(&app, &name, &expression, &say, &sync_empty))
                            }
                        }
                        new_name.set_text("");
                        sync_empty();
                    }
                    Err(error) => say(&error),
                }
            }
        ));

        let content = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(12)
            .margin_top(12)
            .margin_bottom(12)
            .margin_start(12)
            .margin_end(12)
            .build();
        for widget in [
            empty.upcast_ref::<gtk::Widget>(),
            list.upcast_ref(),
            add_list.upcast_ref(),
            add.upcast_ref(),
        ] {
            content.append(widget);
        }
        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .propagate_natural_height(true)
            .child(&content)
            .build();
        toasts.set_child(Some(&scroller));

        let view = adw::ToolbarView::builder().content(&toasts).build();
        view.add_top_bar(&adw::HeaderBar::new());
        let dialog = adw::Dialog::builder()
            .title("Names")
            .content_width(480)
            .content_height(420)
            .child(&view)
            .build();
        dialog.present(Some(&self.window));
    }

    // --- calculations ---

    /// The calculations dialog: every formula in the document, searchable, each row a jump to its
    /// cell — the one question a grid cannot answer, what in here is computed and from what. Plain
    /// arithmetic is listed beside function calls, and the search matches a function name as
    /// readily as an address, so it also answers "where is TODAY used". Rebuilt per keystroke:
    /// `App::calculations` walks formulas already in memory.
    fn explore_calculations(self: &Rc<Self>) {
        let search = gtk::SearchEntry::builder()
            .placeholder_text("Search formulas, addresses, functions")
            .build();
        let summary = gtk::Label::builder().wrap(true).xalign(0.0).build();
        summary.add_css_class("dim-label");
        let list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .build();
        list.add_css_class("boxed-list");

        let dialog = adw::Dialog::builder()
            .title("Calculations")
            .content_width(560)
            .content_height(520)
            .build();

        let refresh: Rc<dyn Fn()> = {
            let (list, summary, search) = (list.clone(), summary.clone(), search.clone());
            let (app, grid, dialog) = (self.app.clone(), self.grid.clone(), dialog.clone());
            Rc::new(move || {
                while let Some(row) = list.first_child() {
                    list.remove(&row);
                }
                let needle = search.text();
                let found: Vec<_> = app
                    .calculations()
                    .into_iter()
                    .filter(|calc| calc.matches(&needle))
                    .collect();
                for calc in &found {
                    let row = adw::ActionRow::builder()
                        .title(glib::markup_escape_text(&headline(calc)))
                        .subtitle(glib::markup_escape_text(&format!(
                            "{} = {}",
                            calc.address(),
                            calc.value
                        )))
                        .activatable(true)
                        .build();
                    row.connect_activated(glib::clone!(
                        #[weak]
                        grid,
                        #[weak]
                        dialog,
                        #[strong(rename_to = sheet)]
                        calc.sheet,
                        #[strong(rename_to = pos)]
                        calc.pos,
                        move |_| {
                            grid.set_sheet(sheet);
                            grid.set_selection(keymap::Selection::at(pos));
                            dialog.close();
                        }
                    ));
                    list.append(&row);
                }
                summary.set_label(&tally(&found));
            })
        };
        refresh();
        search.connect_search_changed(glib::clone!(
            #[strong]
            refresh,
            move |_| refresh()
        ));

        let content = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(12)
            .margin_top(12)
            .margin_bottom(12)
            .margin_start(12)
            .margin_end(12)
            .build();
        for widget in [
            search.upcast_ref::<gtk::Widget>(),
            summary.upcast_ref(),
            list.upcast_ref(),
        ] {
            content.append(widget);
        }
        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .propagate_natural_height(true)
            .child(&content)
            .build();
        let view = adw::ToolbarView::builder().content(&scroller).build();
        view.add_top_bar(&adw::HeaderBar::new());
        dialog.set_child(Some(&view));
        dialog.present(Some(&self.window));
    }

    /// *Replace Formulas with Values* over the selection.
    fn formulas_to_values(self: &Rc<Self>) {
        let used = self.app.used_extent(self.grid.sheet()).unwrap_or((0, 0));
        let (start, end) = grind_sheet::nav::target(self.grid.selection(), used);
        match grind_sheet::verbs::formulas_to_values(&self.app, self.grid.sheet(), start, end) {
            Ok(0) => self.toast("There was no formula in the selection."),
            Ok(n) => self.toast(&format!(
                "{} now plain values.",
                counted(n, "formula is", "formulas are")
            )),
            Err(error) => self.toast(&error.to_string()),
        }
    }

    /// The function list: every function with its plain-English name and what it does — `grind
    /// sheet functions --long`'s columns from the same catalog — searchable; picking one starts
    /// an edit in the active cell seeded `=NAME(`.
    fn explore_functions(self: &Rc<Self>) {
        let search = gtk::SearchEntry::builder()
            .placeholder_text("Search functions by name or plain-English name")
            .build();
        let list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .build();
        list.add_css_class("boxed-list");
        let dialog = adw::Dialog::builder()
            .title("Functions")
            .content_width(560)
            .content_height(520)
            .build();

        let refresh: Rc<dyn Fn()> = {
            let (list, search) = (list.clone(), search.clone());
            let (grid, dialog) = (self.grid.clone(), dialog.clone());
            Rc::new(move || {
                while let Some(row) = list.first_child() {
                    list.remove(&row);
                }
                let wanted = search.text().to_uppercase();
                let mut shown: Vec<_> = grind_sheet::formula::funcs::catalog()
                    .iter()
                    .filter_map(|info| {
                        let friendly = grind_sheet::formula::friendly::signature(info.name)
                            .map(|(head, _)| head)
                            .unwrap_or_else(|| info.name.to_owned());
                        (info.name.to_uppercase().contains(&wanted)
                            || friendly.to_uppercase().contains(&wanted))
                        .then_some((info, friendly))
                    })
                    .collect();
                shown.sort_by_key(|(info, _)| info.name);
                for (info, friendly) in shown {
                    let row = adw::ActionRow::builder()
                        .title(glib::markup_escape_text(&format!(
                            "{} \u{2014} {friendly}",
                            info.name
                        )))
                        .subtitle(glib::markup_escape_text(&format!(
                            "{} ({})",
                            info.brief,
                            grind_sheet::formula::funcs::category(info)
                        )))
                        .activatable(true)
                        .build();
                    row.connect_activated(glib::clone!(
                        #[weak]
                        grid,
                        #[weak]
                        dialog,
                        #[to_owned(rename_to = name)]
                        info.name,
                        move |_| {
                            dialog.close();
                            grid.begin_with(&format!("={name}("));
                        }
                    ));
                    list.append(&row);
                }
            })
        };
        refresh();
        search.connect_search_changed(glib::clone!(
            #[strong]
            refresh,
            move |_| refresh()
        ));

        let content = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(12)
            .margin_top(12)
            .margin_bottom(12)
            .margin_start(12)
            .margin_end(12)
            .build();
        content.append(&search);
        content.append(&list);
        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .propagate_natural_height(true)
            .child(&content)
            .build();
        let view = adw::ToolbarView::builder().content(&scroller).build();
        view.add_top_bar(&adw::HeaderBar::new());
        dialog.set_child(Some(&view));
        dialog.present(Some(&self.window));
    }

    // --- charts ---

    /// The chart dialog (`chart_dialog.rs`): `None` inserts a chart of the selection,
    /// `Some(index)` edits that one.
    fn chart_dialog(self: &Rc<Self>, editing: Option<usize>) {
        chart_dialog::present(&self.window, &self.app, &self.grid, editing);
    }

    /// *Edit a Chart…* — the keyboard's way to a chart, which until this verb had none: a
    /// chart is otherwise reached by double-clicking it. One chart on the sheet is edited
    /// straight away; several are offered by name first.
    fn choose_chart(self: &Rc<Self>) {
        let sheet = self.grid.sheet();
        let charts = self.app.charts(sheet).unwrap_or_default();
        match charts.len() {
            0 => self.toast("There is no chart on this sheet"),
            1 => self.chart_dialog(Some(0)),
            _ => {
                let list = gtk::ListBox::builder()
                    .selection_mode(gtk::SelectionMode::None)
                    .build();
                list.add_css_class("boxed-list");
                let dialog = adw::AlertDialog::new(Some("Edit a Chart"), None);
                for (index, chart) in charts.iter().enumerate() {
                    let row = adw::ActionRow::builder()
                        .title(glib::markup_escape_text(&chart_name(chart)))
                        .subtitle(glib::markup_escape_text(
                            &chart.categories.clone().unwrap_or_default(),
                        ))
                        .activatable(true)
                        .build();
                    row.connect_activated(glib::clone!(
                        #[strong(rename_to = ui)]
                        self,
                        #[weak]
                        dialog,
                        move |_| {
                            dialog.close();
                            ui.chart_dialog(Some(index));
                        }
                    ));
                    list.append(&row);
                }
                dialog.set_extra_child(Some(&list));
                dialog.add_response("cancel", "Cancel");
                dialog.set_close_response("cancel");
                dialog.present(Some(&self.window));
            }
        }
    }

    /// Deleting a chart is immediate, with an Undo toast — the inverse carries the whole
    /// chart, exactly as deleting a sheet does.
    fn delete_chart(self: &Rc<Self>, index: usize) {
        match self.app.remove_chart(self.grid.sheet(), index) {
            Ok(()) => self.undoable_toast("Deleted chart"),
            Err(error) => self.toast(&error.to_string()),
        }
    }

    // --- sheets ---

    fn add_sheet(self: &Rc<Self>) {
        let name = self.app.fresh_sheet_name();
        match self.app.add_sheet(&name) {
            Ok(index) => self.grid.set_sheet(index),
            Err(error) => self.toast(&error.to_string()),
        }
    }

    fn rename_sheet(self: &Rc<Self>) {
        let sheet = self.grid.sheet();
        let Ok(current) = self.app.sheet_name(sheet) else {
            return;
        };
        let entry = gtk::Entry::builder()
            .text(&current)
            .activates_default(true)
            .build();
        let dialog = adw::AlertDialog::new(Some("Rename Sheet"), None);
        dialog.set_extra_child(Some(&entry));
        dialog.add_response("cancel", "Cancel");
        dialog.add_response("rename", "Rename");
        dialog.set_response_appearance("rename", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("rename"));
        dialog.set_close_response("cancel");
        dialog.choose(
            &self.window,
            gio::Cancellable::NONE,
            glib::clone!(
                #[strong(rename_to = ui)]
                self,
                move |response| {
                    if response != "rename" {
                        return;
                    }
                    // A rename carries every formula, named expression and chart range with it, in
                    // one undo step; the toast says how many, since an edit nobody saw happen is
                    // one they cannot trust.
                    match ui.app.rename_sheet(sheet, entry.text().trim()) {
                        Ok(0) => {}
                        Ok(rewritten) => {
                            ui.toast(&format!("{rewritten} reference(s) rewritten"));
                        }
                        Err(error) => ui.toast(&error.to_string()),
                    }
                }
            ),
        );
    }

    /// Format the selection as a table: autofilter, banded rows, an optional totals row and a
    /// name, in [`grind_sheet::App::format_table`]'s one undo step. There is no table object to
    /// reopen (`sheet/src/table_format.rs`), so this is only ever the apply step; run again over a
    /// styled range, it restyles.
    fn format_table_dialog(self: &Rc<Self>) {
        let (start, mut end) = self.grid.selection().rect();
        let sheet = self.grid.sheet();
        // A single cell is a click, not a range: format the used table around it, the same
        // rule `Grid::toggle_filter` uses.
        if start == end
            && let Ok((rows, cols)) = self.app.used_extent(sheet)
        {
            end = grind_sheet::Pos::new(rows.saturating_sub(1), cols.saturating_sub(1));
        }
        if end.row <= start.row {
            return self.toast("Select the rows to format, including their headings");
        }

        // A boxed list of Adwaita rows, the shape every other dialog in this window has — a
        // switch, a choice and an entry, each a row a reader can take in at a glance.
        let header = adw::SwitchRow::builder()
            .title("Header row")
            .subtitle("The first row names the columns")
            .active(true)
            .build();
        // The totals row's aggregate, "None" first — one dropdown, since "no totals row" is one
        // more entry. The labels are `TotalsFunction::label`.
        let mut totals_labels = vec!["None".to_owned()];
        totals_labels.extend(
            grind_sheet::TotalsFunction::ALL
                .iter()
                .map(|f| f.label().to_owned()),
        );
        let totals = adw::ComboRow::builder()
            .title("Totals row")
            .model(&gtk::StringList::new(
                &totals_labels.iter().map(String::as_str).collect::<Vec<_>>(),
            ))
            .build();
        let name = adw::EntryRow::builder()
            .title("Name (optional)")
            .activates_default(true)
            .build();
        let body = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .build();
        body.add_css_class("boxed-list");
        body.append(&header);
        body.append(&totals);
        body.append(&name);

        let dialog = adw::AlertDialog::new(Some("Format as Table"), None);
        dialog.set_extra_child(Some(&body));
        dialog.add_response("cancel", "Cancel");
        dialog.add_response("format", "Format");
        dialog.set_response_appearance("format", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("format"));
        dialog.set_close_response("cancel");
        dialog.choose(
            &self.window,
            gio::Cancellable::NONE,
            glib::clone!(
                #[strong(rename_to = ui)]
                self,
                move |response| {
                    if response != "format" {
                        return;
                    }
                    let typed = name.text();
                    // Row 0 is "None"; every row after it is `TotalsFunction::ALL` in order.
                    let chosen = totals.selected() as usize;
                    let options = grind_sheet::TableOptions {
                        header: header.is_active(),
                        totals: chosen
                            .checked_sub(1)
                            .and_then(|i| grind_sheet::TotalsFunction::ALL.get(i).copied()),
                        name: (!typed.trim().is_empty()).then(|| typed.trim().to_owned()),
                    };
                    match ui.app.format_table(sheet, start, end, options) {
                        Ok(settled) => ui.toast(&format!("Formatted as table “{settled}”")),
                        Err(error) => ui.toast(&error.to_string()),
                    }
                }
            ),
        );
    }

    /// Deleting is immediate, with an Undo toast — the inverse carries the whole sheet, so
    /// taking it back really does bring the cells with it.
    fn delete_sheet(self: &Rc<Self>) {
        let sheet = self.grid.sheet();
        let name = self.app.sheet_name(sheet).unwrap_or_default();
        match self.app.remove_sheet(sheet) {
            Ok(()) => {
                self.grid.set_sheet(sheet.saturating_sub(1));
                self.undoable_toast(&format!("Deleted “{name}”"));
            }
            Err(error) => self.toast(&error.to_string()),
        }
    }

    // --- closing ---

    /// Ask before throwing work away. `then` runs when there is nothing to lose, or once
    /// the user has said so.
    fn confirm_discard(self: &Rc<Self>, then: impl Fn() + 'static) {
        if !self.dirty.get() {
            then();
            return;
        }
        let dialog = adw::AlertDialog::new(
            Some("Discard unsaved changes?"),
            Some("This document has changes that have not been saved."),
        );
        dialog.add_response("cancel", "Cancel");
        dialog.add_response("discard", "Discard");
        dialog.set_response_appearance("discard", adw::ResponseAppearance::Destructive);
        dialog.set_close_response("cancel");
        dialog.choose(&self.window, gio::Cancellable::NONE, move |response| {
            if response == "discard" {
                then();
            }
        });
    }

    fn confirm_close(self: &Rc<Self>) -> glib::Propagation {
        if !self.dirty.get() || self.closing.get() {
            return glib::Propagation::Proceed;
        }
        let dialog = adw::AlertDialog::new(
            Some("Save changes before closing?"),
            Some("This document has changes that have not been saved."),
        );
        dialog.add_response("cancel", "Cancel");
        dialog.add_response("discard", "Discard");
        dialog.add_response("save", "Save");
        dialog.set_response_appearance("discard", adw::ResponseAppearance::Destructive);
        dialog.set_response_appearance("save", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("save"));
        dialog.set_close_response("cancel");
        dialog.choose(
            &self.window,
            gio::Cancellable::NONE,
            glib::clone!(
                #[strong(rename_to = ui)]
                self,
                move |response| match response.as_str() {
                    // The latch: the close is resumed by whichever of these finishes, and a
                    // failed save clears it so the window stays open with the work in it.
                    "save" => {
                        ui.closing.set(true);
                        ui.save();
                    }
                    "discard" => {
                        ui.dirty.set(false);
                        ui.window.close();
                    }
                    _ => {}
                }
            ),
        );
        glib::Propagation::Stop
    }

    /// The shortcuts dialog, built from the table the window wires up — titles included — so it
    /// cannot list a binding the keyboard lacks, plus the grid's own keys (`keymap.rs`), which
    /// have no `GAction`.
    fn shortcuts(&self) {
        let window = gtk::ShortcutsWindow::builder()
            .transient_for(&self.window)
            .modal(true)
            .build();

        let group = gtk::ShortcutsGroup::builder().title("General").build();
        // Everything with a key and a name, in the table's own order. A verb reached only
        // from a menu has no row here, which is what `accels.is_empty()` says.
        for row in palette_rows() {
            if row.accels.is_empty() || row.title.is_empty() {
                continue;
            }
            group.add_shortcut(
                &gtk::ShortcutsShortcut::builder()
                    .title(row.title)
                    .accelerator(row.accels.join(" "))
                    .build(),
            );
        }
        let navigation = gtk::ShortcutsGroup::builder()
            .title("Navigation & Editing")
            .build();
        for (accelerator, title) in [
            ("Left Right Up Down", "Move selection"),
            (
                "<Control>Left <Control>Right <Control>Up <Control>Down",
                "Jump to data edge",
            ),
            (
                "<Shift>Left <Shift>Right <Shift>Up <Shift>Down",
                "Extend selection",
            ),
            ("<Control>a", "Select all"),
            ("Tab ISO_Left_Tab", "Move within a row"),
            ("Return <Shift>Return", "Move within a column"),
            ("Delete BackSpace", "Clear selection"),
            ("F2", "Edit cell"),
            ("F4", "Cycle $ in a reference"),
            ("Escape", "Cancel edit"),
            ("<Control>c <Control>x <Control>v", "Copy, cut, paste"),
            ("<Control><Shift>c", "Copy value"),
            ("<Control>d", "Fill down"),
            ("<Control>r", "Fill right"),
        ] {
            navigation.add_shortcut(
                &gtk::ShortcutsShortcut::builder()
                    .title(title)
                    .accelerator(accelerator)
                    .build(),
            );
        }

        let section = gtk::ShortcutsSection::builder()
            .section_name("main")
            .build();
        section.add_group(&group);
        section.add_group(&navigation);
        window.add_section(&section);
        window.present();
    }

    fn about(&self) {
        let about = adw::AboutDialog::builder()
            .application_name("Sheet")
            .application_icon(APP_ID)
            .developer_name("Florian Wilhelm")
            .version(env!("CARGO_PKG_VERSION"))
            .website("https://github.com/fwilhe2/grind")
            .license_type(gtk::License::Agpl30)
            .comments("An ODF-native spreadsheet.")
            .debug_info(grind_sheet::build_info::describe(
                "grind-sheet-gtk",
                env!("CARGO_PKG_VERSION"),
            ))
            .build();
        licences::add_to(&about);
        about.present(Some(&self.window));
    }
}

type Handler = fn(&Rc<Ui>);

/// One verb this window has: a `win.` action, its name, and what runs it. **The one table**, with
/// four readers: `wire` builds the actions and accelerators, the menus name them, the shortcuts
/// window lists them and the palette searches them. A verb that is here is in the palette, which
/// makes "a feature does not get a button" structural.
struct Verb {
    /// The action's name, without the `win.` prefix.
    name: &'static str,
    accels: &'static [&'static str],
    /// How the palette and the shortcuts window name it. `""` for the handful that exist only
    /// so a widget can reach them and that nobody would ever go looking for by name.
    title: &'static str,
    group: &'static str,
    run: Handler,
}

/// The **readings** — ways of looking at the document (`doc/view-modes.md`, `doc/dsl.md` §6).
/// Stateful, so `wire` builds them by hand; named here so the View menu, the palette and the
/// shortcuts window read the list the wiring does. None writes a byte.
const READINGS: [(&str, &str, &str); 6] = [
    (
        "friendly-formulas",
        "Read Formulas as Sentences",
        "", // No accelerator: it is a preference, not a gesture.
    ),
    ("show-names", "Show Where Names Live", "<Control><Shift>n"),
    ("show-roles", "Show What Each Cell Is", "<Control><Shift>r"),
    ("show-formulas", "Show Formulas Instead of Results", ""),
    ("show-source", "Show the Source", "<Control><Shift>u"),
    // Shift+F7 is *Automatic Spell Checking* in LibreOffice, so the key a person already has
    // in their fingers turns it off here too.
    ("spell-check", "Check Spelling", "<Shift>F7"),
];

/// The one action a menu reaches **with a target** — the Spelling Language submenu's radio
/// items, one per `grind_spell::Setting` — and so the one the palette, which activates an action
/// with none, cannot offer. Check Spelling is its palette half.
const TARGETED: [&str; 1] = ["spell-language"];

/// Bind a reading's accelerator as [`READINGS`] spells it, so the palette, the shortcuts window
/// and the keyboard agree about what Ctrl+Shift+N does.
fn accelerate(application: &adw::Application, name: &str) {
    for (verb, _, accel) in READINGS {
        if verb == name && !accel.is_empty() {
            application.set_accels_for_action(&format!("win.{name}"), &[accel]);
        }
    }
}

/// Every verb as the palette takes them — the plain ones and the readings. `OnceLock` because
/// [`palette::present`] holds the rows for the dialog's life, and `actions()` builds from function
/// pointers, which a `const` cannot hold.
fn palette_rows() -> &'static [palette::Row] {
    static ROWS: std::sync::OnceLock<Vec<palette::Row>> = std::sync::OnceLock::new();
    ROWS.get_or_init(|| {
        actions()
            .into_iter()
            .filter(|verb| !verb.title.is_empty())
            .map(|verb| palette::Row {
                name: verb.name,
                title: verb.title,
                group: verb.group,
                accels: verb.accels,
            })
            .chain(READINGS.iter().map(|(name, title, accel)| palette::Row {
                name,
                title,
                group: "View",
                // A slice of one, or none — the same shape `Verb::accels` has, built from the
                // single accelerator a reading carries.
                accels: match accel.is_empty() {
                    true => &[],
                    false => std::slice::from_ref(accel),
                },
            }))
            .collect()
    })
}

/// One name as a row: its definition editable in place, and buttons that rename, inline or remove
/// it. `set_name` over the same name replaces it, so an edit is a definition. **Rename** and
/// **Inline** carry every use of the name (`App::rename_name`, `App::inline_name`), each one undo
/// step saying how many uses it rewrote. The row's name is a cell, since a rename changes it under
/// the closures.
fn name_row(
    app: &Arc<App>,
    name: &str,
    expression: &str,
    say: &Rc<dyn Fn(&str)>,
    sync_empty: &Rc<dyn Fn()>,
) -> adw::EntryRow {
    let current = Rc::new(std::cell::RefCell::new(name.to_owned()));
    let row = adw::EntryRow::builder()
        .title(name)
        .text(definition_text(expression))
        .show_apply_button(true)
        .build();
    row.connect_apply(glib::clone!(
        #[strong]
        app,
        #[strong]
        say,
        #[strong]
        current,
        move |row| {
            if let Err(error) = define(&app, &current.borrow(), &row.text()) {
                say(&error);
            }
        }
    ));

    let rename = gtk::Button::from_icon_name("document-edit-symbolic");
    rename.set_tooltip_text(Some("Rename, carrying every use"));
    rename.set_valign(gtk::Align::Center);
    rename.add_css_class("flat");
    rename.connect_clicked(glib::clone!(
        #[strong]
        app,
        #[strong]
        say,
        #[strong]
        current,
        #[weak]
        row,
        move |_| {
            let entry = gtk::Entry::builder()
                .text(current.borrow().as_str())
                .activates_default(true)
                .build();
            let dialog = adw::AlertDialog::new(Some("Rename Name"), None);
            dialog.set_body("Every formula and name that uses it follows.");
            dialog.set_extra_child(Some(&entry));
            dialog.add_response("cancel", "Cancel");
            dialog.add_response("rename", "Rename");
            dialog.set_response_appearance("rename", adw::ResponseAppearance::Suggested);
            dialog.set_default_response(Some("rename"));
            dialog.set_close_response("cancel");
            dialog.choose(
                &row,
                gio::Cancellable::NONE,
                glib::clone!(
                    #[strong]
                    app,
                    #[strong]
                    say,
                    #[strong]
                    current,
                    #[weak]
                    row,
                    move |response| {
                        if response != "rename" {
                            return;
                        }
                        let to = entry.text();
                        match app.rename_name(&current.borrow(), to.trim()) {
                            Ok(uses) => {
                                *current.borrow_mut() = to.trim().to_owned();
                                row.set_title(to.trim());
                                say(&format!("Renamed — {uses} use(s) rewritten"));
                            }
                            Err(error) => say(&error.to_string()),
                        }
                    }
                ),
            );
        }
    ));
    row.add_suffix(&rename);

    let inline = gtk::Button::from_icon_name("go-jump-symbolic");
    inline.set_tooltip_text(Some("Inline into every use, then delete the name"));
    inline.set_valign(gtk::Align::Center);
    inline.add_css_class("flat");
    inline.connect_clicked(glib::clone!(
        #[strong]
        app,
        #[strong]
        say,
        #[strong]
        sync_empty,
        #[strong]
        current,
        #[weak]
        row,
        move |_| match app.inline_name(&current.borrow()) {
            Ok(uses) => {
                if let Some(list) = row.parent().and_downcast::<gtk::ListBox>() {
                    list.remove(&row);
                }
                sync_empty();
                say(&format!("Inlined — {uses} use(s) rewritten"));
            }
            Err(error) => say(&error.to_string()),
        }
    ));
    row.add_suffix(&inline);

    let delete = gtk::Button::from_icon_name("user-trash-symbolic");
    delete.set_tooltip_text(Some("Delete"));
    delete.set_valign(gtk::Align::Center);
    delete.add_css_class("flat");
    delete.connect_clicked(glib::clone!(
        #[strong]
        app,
        #[strong]
        sync_empty,
        #[strong]
        current,
        #[weak]
        row,
        move |_| {
            // A formula that mentions a deleted name goes stale rather than being rewritten
            // — `App::clear_name`'s documented answer, and the banner counts it.
            app.clear_name(&current.borrow());
            if let Some(list) = row.parent().and_downcast::<gtk::ListBox>() {
                list.remove(&row);
            }
            sync_empty();
        }
    ));
    row.add_suffix(&delete);
    row
}

/// A chart as a list names it: its own title, or what kind of chart of what it is.
fn chart_name(chart: &grind_sheet::Chart) -> String {
    match &chart.title {
        Some(title) => title.clone(),
        None => format!(
            "{} chart of {}",
            match chart.kind {
                grind_sheet::ChartKind::Bar => "Bar",
                grind_sheet::ChartKind::Line => "Line",
                grind_sheet::ChartKind::Pie => "Pie",
            },
            chart
                .series
                .iter()
                .map(|s| s.values.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

/// What a calculation's row says first: the friendly rendering of its formula, else the formula —
/// `Round(Value: A1; Digits: 2)` says what a cell does quicker, and the address line still names
/// the cell.
fn headline(calc: &grind_sheet::Calculation) -> String {
    chrome::friendly_line(&calc.formula).unwrap_or_else(|| calc.formula.clone())
}

/// `1 cell became an error` / `3 cells became errors` — the two spellings, by count.
/// "1 cell(s)" is the one string shape that reads like the printf that made it.
fn counted(n: usize, one: &str, many: &str) -> String {
    match n {
        1 => format!("1 {one}"),
        n => format!("{n} {many}"),
    }
}

/// The line above the list: how many were found, and which functions they call.
fn tally(found: &[grind_sheet::Calculation]) -> String {
    if found.is_empty() {
        return "Nothing here is calculated. A cell starting with = is.".to_owned();
    }
    let counted = match found.len() {
        1 => "1 calculation".to_owned(),
        n => format!("{n} calculations"),
    };
    let functions = grind_sheet::function_tally(found);
    match functions.is_empty() {
        true => counted,
        false => format!(
            "{counted} — {}",
            functions
                .iter()
                .map(|(name, count)| format!("{name} ×{count}"))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

/// The row for a name, if the list already has one — names are case-insensitive, as they
/// are everywhere else.
fn row_named(list: &gtk::ListBox, name: &str) -> Option<adw::EntryRow> {
    let mut child = list.first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        if let Ok(row) = widget.downcast::<adw::EntryRow>()
            && row.title().eq_ignore_ascii_case(name.trim())
        {
            return Some(row);
        }
    }
    None
}

/// Define `name` as `target`, returning what was stored. `a1::definition` is the shared
/// rule — a leading `=` is a formula, anything else an address — and `App::set_name` does
/// every refusal, so its message is what a user sees.
fn define(app: &App, name: &str, target: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("A name is needed".to_owned());
    }
    let expression = a1::definition(app, target).map_err(|e| e.to_string())?;
    app.set_name(name, &expression)
        .map(|()| expression)
        .map_err(|e| e.to_string())
}

/// A stored definition in the form the dialog takes back.
///
/// A reference arrives bracketed, which `a1::definition` accepts as it stands; anything else
/// is a *formula* and needs the `=` that says so, or retyping what was shown would be read
/// as an address.
fn definition_text(expression: &str) -> String {
    match expression.starts_with('[') {
        true => expression.to_owned(),
        false => format!("={expression}"),
    }
}

/// Every verb, in the order the palette offers them with nothing typed — grouped by what they
/// are about rather than alphabetically, because that empty list is one to *read*.
fn actions() -> Vec<Verb> {
    // A row per verb, which is one line of noise per verb less than a struct literal.
    const fn verb(
        name: &'static str,
        accels: &'static [&'static str],
        title: &'static str,
        group: &'static str,
        run: Handler,
    ) -> Verb {
        Verb {
            name,
            accels,
            title,
            group,
            run,
        }
    }
    vec![
        // --- the document ---
        verb("new", &["<Control>n"], "New Document", "Document", |ui| {
            ui.new_document()
        }),
        verb("open", &["<Control>o"], "Open…", "Document", |ui| {
            ui.open()
        }),
        verb("save", &["<Control>s"], "Save", "Document", |ui| ui.save()),
        verb(
            "save-as",
            &["<Control><Shift>s"],
            "Save As…",
            "Document",
            |ui| ui.save_as(),
        ),
        // The one non-ODF format (`doc/not-doing.md` §2), and the only interchange this suite
        // has. No accelerator: neither is a verb anybody presses twice a minute, and the
        // palette is the growth valve this window put verbs in.
        verb("csv-import", &[], "Import CSV…", "Document", |ui| {
            ui.import_csv()
        }),
        verb(
            "csv-import-with",
            &[],
            "Import CSV with Options…",
            "Document",
            |ui| ui.import_csv_with(),
        ),
        verb("csv-export", &[], "Export CSV…", "Document", |ui| {
            ui.export_csv()
        }),
        verb("undo", &["<Control>z"], "Undo", "Document", |ui| {
            ui.app.undo();
        }),
        verb(
            "redo",
            &["<Control><Shift>z", "<Control>y"],
            "Redo",
            "Document",
            |ui| {
                ui.app.redo();
            },
        ),
        verb("recalc", &["F9"], "Recalculate", "Document", |ui| {
            ui.recalculate()
        }),
        // F8 is the "next problem" key every IDE has; nothing in the grid's own key map
        // claims it. `doc/dsl.md` §4.3 — the rules are the core's, this is a list in front
        // of them.
        verb("lint", &["F8"], "Check the Document", "Document", |ui| {
            lint::present(&ui.window, &ui.app, &ui.grid)
        }),
        // Ctrl+F and Ctrl+H, the two keys every spreadsheet gives them — Ctrl+H is the same
        // bar with its replace row already open, not a second dialog.
        verb("find", &["<Control>f"], "Find…", "Document", |ui| {
            ui.find.open(false)
        }),
        verb(
            "find-replace",
            &["<Control>h"],
            "Find and Replace…",
            "Document",
            |ui| ui.find.open(true),
        ),
        verb(
            "calculations",
            &["<Control><Shift>f"],
            "Find a Calculation…",
            "Document",
            |ui| ui.explore_calculations(),
        ),
        verb("names", &[], "Names…", "Document", |ui| ui.manage_names()),
        // A palette verb rather than a format-bar control: a rule is something the sheet has,
        // not a property of the selection (`doc/conditional-format.md` §4).
        verb(
            "conditional-formats",
            &[],
            "Conditional Formatting…",
            "Document",
            |ui| {
                rules::present(
                    &ui.window,
                    &ui.app,
                    ui.grid.sheet(),
                    ui.grid.selection().rect(),
                )
            },
        ),
        // Each formula in the selection dropped, the cell keeping the value it last computed —
        // `verbs::formulas_to_values`, one undo step per formula (the core has no range form).
        verb(
            "formulas-to-values",
            &[],
            "Replace Formulas with Values",
            "Document",
            |ui| ui.formulas_to_values(),
        ),
        verb("functions", &[], "Functions…", "Document", |ui| {
            ui.explore_functions()
        }),
        verb(
            "spelling-language",
            &[],
            "Spelling Language…",
            "View",
            |ui| ui.spelling_language_dialog(),
        ),
        verb(
            "personal-dictionary",
            &[],
            "Personal Dictionary…",
            "View",
            |ui| {
                let again = ui.clone();
                words::present(&ui.window, move || again.check_spelling(false));
            },
        ),
        verb(
            "document-settings",
            &[],
            "Document Settings…",
            "Document",
            |ui| settings::present(&ui.window, &ui.app),
        ),
        // --- the selection ---
        //
        // The clipboard four have no accelerator here: `keymap.rs` owns Ctrl+C/X/V and Delete in
        // the grid, and two bindings for one key is how a shortcut does two things. They are
        // `win.` actions so the context menu and the palette reach them.
        verb("copy", &[], "Copy", "Selection", |ui| ui.grid.copy(false)),
        verb("cut", &[], "Cut", "Selection", |ui| ui.grid.copy(true)),
        verb("paste", &[], "Paste", "Selection", |ui| ui.grid.paste()),
        verb("copy-value", &[], "Copy Value", "Selection", |ui| {
            ui.grid.copy_value()
        }),
        verb("clear", &[], "Clear Contents", "Selection", |ui| {
            ui.grid.clear()
        }),
        verb("merge", &[], "Merge Cells", "Selection", |ui| {
            ui.grid.merge(true)
        }),
        verb("unmerge", &[], "Unmerge Cells", "Selection", |ui| {
            ui.grid.merge(false)
        }),
        verb(
            "select-all",
            &[],
            "Select the Whole Sheet",
            "Selection",
            |ui| ui.grid.select_all(),
        ),
        verb("fill-down", &[], "Fill Down", "Selection", |ui| {
            ui.grid.fill(keymap::Dir::Down)
        }),
        verb("fill-right", &[], "Fill Right", "Selection", |ui| {
            ui.grid.fill(keymap::Dir::Right)
        }),
        // Ctrl+Shift+L is the filter key both other spreadsheets use, and nothing in the
        // grid's own key map claims it.
        verb(
            "filter",
            &["<Control><Shift>l"],
            "Filter Rows",
            "Selection",
            |ui| ui.grid.toggle_filter(),
        ),
        verb("chart-insert", &[], "Insert a Chart…", "Selection", |ui| {
            ui.chart_dialog(None)
        }),
        verb("chart-edit", &[], "Edit a Chart…", "Selection", |ui| {
            ui.choose_chart()
        }),
        verb(
            "format-table",
            &[],
            "Format as Table…",
            "Selection",
            |ui| ui.format_table_dialog(),
        ),
        verb(
            "explain-formula",
            &["<Control><Shift>e"],
            "Explain This Formula",
            "Selection",
            |ui| ui.formula_bar.explain.popup(),
        ),
        // --- the sheets ---
        verb("sheet-add", &[], "Add a Sheet", "Sheets", |ui| {
            ui.add_sheet()
        }),
        verb("sheet-rename", &[], "Rename This Sheet…", "Sheets", |ui| {
            ui.rename_sheet()
        }),
        verb("sheet-delete", &[], "Delete This Sheet", "Sheets", |ui| {
            ui.delete_sheet()
        }),
        // --- the view ---
        //
        // Both spellings of the key, because a keyboard's `+` is `plus` with Shift and
        // `equal` without, and a user pressing Ctrl and the key next to Backspace means one
        // thing by it either way.
        verb(
            "zoom-in",
            &["<Control>plus", "<Control>equal", "<Control>KP_Add"],
            "Zoom In",
            "View",
            |ui| ui.grid.set_zoom(ui.grid.zoom() * grid::ZOOM_STEP),
        ),
        verb(
            "zoom-out",
            &["<Control>minus", "<Control>KP_Subtract"],
            "Zoom Out",
            "View",
            |ui| ui.grid.set_zoom(ui.grid.zoom() / grid::ZOOM_STEP),
        ),
        verb(
            "zoom-reset",
            &["<Control>0", "<Control>KP_0"],
            "Normal Size",
            "View",
            |ui| ui.grid.set_zoom(1.0),
        ),
        verb("autofit-all", &[], "Fit Content to Cells", "View", |ui| {
            ui.grid.autofit_all()
        }),
        verb("fit-cols", &[], "Fit Column Width", "View", |ui| {
            ui.grid.fit_selection(false)
        }),
        verb("fit-rows", &[], "Fit Row Height", "View", |ui| {
            ui.grid.fit_selection(true)
        }),
        // --- the window ---
        //
        // The palette cannot list itself: a row that opens the box it is being read in is a
        // row that does nothing, which is why this one has no title.
        verb("palette", &["<Control>k"], "", "Window", |ui| {
            palette::present(&ui.window, palette_rows())
        }),
        verb(
            "shortcuts",
            &["<Control>question"],
            "Keyboard Shortcuts",
            "Window",
            |ui| ui.shortcuts(),
        ),
        verb("about", &[], "About Sheet", "Window", |ui| ui.about()),
    ]
}

/// The primary menu: **the document and the window, nothing about the selection** — the HIG's
/// rule. Cell verbs are in the grid's context menu, how the document is shown in the View menu,
/// the sheet verbs on the tab. What is left is the file verbs, what is done to a document as a
/// whole, and the window items. *Find a Command* is here because a menu is where a reader goes
/// when they do not know where something is.
fn primary_menu() -> gio::Menu {
    let menu = gio::Menu::new();
    let files = gio::Menu::new();
    files.append(Some("New"), Some("win.new"));
    files.append(Some("Open…"), Some("win.open"));
    files.append(Some("Save"), Some("win.save"));
    files.append(Some("Save As…"), Some("win.save-as"));
    menu.append_section(None, &files);

    // CSV is a *file* verb and belongs in this menu for the same reason the four above do —
    // its own section, because importing is not another way of saving the document.
    let interchange = gio::Menu::new();
    interchange.append(Some("Import CSV…"), Some("win.csv-import"));
    interchange.append(Some("Export CSV…"), Some("win.csv-export"));
    menu.append_section(None, &interchange);

    let document = gio::Menu::new();
    document.append(Some("Find and Replace…"), Some("win.find-replace"));
    document.append(Some("Recalculate"), Some("win.recalc"));
    document.append(Some("Check the Document"), Some("win.lint"));
    document.append(Some("Find a Calculation…"), Some("win.calculations"));
    document.append(Some("Names…"), Some("win.names"));
    document.append(Some("Document Settings…"), Some("win.document-settings"));
    menu.append_section(None, &document);

    let rest = gio::Menu::new();
    rest.append(Some("Find a Command…"), Some("win.palette"));
    rest.append(Some("Keyboard Shortcuts"), Some("win.shortcuts"));
    rest.append(Some("About Sheet"), Some("win.about"));
    menu.append_section(None, &rest);
    menu
}

/// The open filter, in the words a user knows the format by; nothing user-facing names another
/// program (`CONTRIBUTING.md`). One filter for all three extensions: packaged, flat and projected
/// are one document to everyone but the writer.
fn spreadsheet_filters() -> gio::ListStore {
    let filter = gtk::FileFilter::new();
    filter.set_name(Some("OpenDocument Spreadsheet"));
    filter.add_pattern("*.fods");
    filter.add_pattern("*.ods");
    // The third physical form (`doc/dsl.md` §9). The window needed no other change to open
    // one — `App::open_bytes` sniffs it — but a file a dialog filters out is a file the user
    // cannot reach, so the pattern is not optional.
    filter.add_pattern("*.grind");

    let filters = gio::ListStore::new::<gtk::FileFilter>();
    // A workbook and a CSV are imported, so each is a filter of its own, named as a
    // person knows it; everything together first, so the default shows every file this window
    // opens.
    let mut patterns = vec!["*.fods", "*.ods", "*.grind"];
    let mut imports = Vec::new();
    if cfg!(feature = "xlsx") {
        let workbooks = gtk::FileFilter::new();
        workbooks.set_name(Some("Excel Workbook"));
        for pattern in ["*.xlsx", "*.xlsm"] {
            workbooks.add_pattern(pattern);
            patterns.push(pattern);
        }
        imports.push(workbooks);
    }
    let delimited = gtk::FileFilter::new();
    delimited.set_name(Some("CSV and TSV"));
    for pattern in ["*.csv", "*.tsv", "*.tab"] {
        delimited.add_pattern(pattern);
        patterns.push(pattern);
    }
    imports.push(delimited);
    let everything = gtk::FileFilter::new();
    everything.set_name(Some("All Spreadsheets"));
    for pattern in patterns {
        everything.add_pattern(pattern);
    }
    filters.append(&everything);
    filters.append(&filter);
    for import in &imports {
        filters.append(import);
    }
    filters
}

/// What an import opens: both spellings of the one format — the delimiter is a field of
/// `csv::Dialect` — and `.txt`, which many exports are called, since the delimiter is sniffed from
/// the content.
fn csv_filters() -> gio::ListStore {
    let filter = gtk::FileFilter::new();
    filter.set_name(Some("Delimited text"));
    for pattern in ["*.csv", "*.tsv", "*.tab", "*.txt"] {
        filter.add_pattern(pattern);
    }
    let filters = gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&filter);
    filters
}

/// Exporting is the other direction, and here the spellings **are** a choice: the name decides the
/// delimiter (`csv::Dialect::for_name`). Comma first, as the verb says.
fn csv_save_filters() -> gio::ListStore {
    let filters = gio::ListStore::new::<gtk::FileFilter>();
    for (name, pattern) in [
        ("Comma-separated values", "*.csv"),
        ("Tab-separated values", "*.tsv"),
    ] {
        let filter = gtk::FileFilter::new();
        filter.set_name(Some(name));
        filter.add_pattern(pattern);
        filters.append(&filter);
    }
    filters
}

/// Saving has an answer (`doc/flat-first.md`): three filters, flat first, so the default writes
/// the form that diffs. The projection is last: it diffs better still, but nothing else reads it.
fn spreadsheet_save_filters() -> gio::ListStore {
    let filters = gio::ListStore::new::<gtk::FileFilter>();
    for (name, pattern) in [
        ("OpenDocument Spreadsheet (flat XML)", "*.fods"),
        ("OpenDocument Spreadsheet (package)", "*.ods"),
        ("Grind projection", "*.grind"),
    ] {
        let filter = gtk::FileFilter::new();
        filter.set_name(Some(name));
        filter.add_pattern(pattern);
        filters.append(&filter);
    }
    filters
}

/// A token crossing from wherever a change was made to the main loop.
struct Bridge(async_channel::Sender<()>);

impl Observer for Bridge {
    fn changed(&self) {
        // Unbounded and coalesced on the other side, so a full channel cannot happen and a
        // dropped receiver is just a window that has gone away.
        let _ = self.0.try_send(());
    }
}

/// Recent files, the native way: `gtk::FileDialog`'s own "Recent" section already reads
/// `GtkRecentManager`, so opening or saving a document only has to register it there — no
/// custom "Open Recent" menu to build or keep in sync.
fn remember_recent(path: &Path) {
    gtk::RecentManager::default().add_item(&gio::File::for_path(path).uri());
}

/// An empty document as bytes — what *New* opens and a window started without a file shows. It
/// states a locale: `$GRIND_LOCALE` or the config file's, else the desktop's (`LC_ALL`,
/// `LC_NUMERIC`, `LANG`), so a spreadsheet begun on a German desktop spells `1234,5`
/// (`doc/ods-format.md` §5.2). `sheet new` states none unless told: a script's output should not
/// depend on whose shell ran it.
fn fresh_document() -> Vec<u8> {
    let doc = grind_sheet::Document {
        locale: grind_sheet::locale::from_environment().or_else(grind_sheet::locale::from_desktop),
        ..Default::default()
    };
    grind_sheet::write_bytes(&doc, Form::Flat).expect("an empty document always writes")
}

/// The document's name, or what an unsaved one is called until it has one.
fn document_name(path: Option<&Path>) -> String {
    path.and_then(|p| p.file_name()).map_or_else(
        || "Untitled".to_owned(),
        |name| name.to_string_lossy().into_owned(),
    )
}

/// What Save As puts in its name field for a document with no path: the flat extension
/// (`doc/flat-first.md`), so the keystroke most likely left alone writes a file that diffs. A
/// document with a path keeps its own name and form; nothing converts a document behind somebody's
/// back.
fn save_name(path: Option<&Path>) -> String {
    match path {
        Some(_) => document_name(path),
        None => format!("Untitled.{}", Form::Flat.extension(KIND)),
    }
}

/// Draw one frame to a PNG and quit — `--render-to`. It waits a beat for the window to be mapped
/// and allocated, or the grid would not know how many cells fit and the image would be blank.
fn render_once(window: &adw::ApplicationWindow, target: PathBuf) {
    let window = window.clone();
    glib::timeout_add_local_once(std::time::Duration::from_millis(600), move || {
        let width = window.width();
        let height = window.height();
        let paintable = gtk::WidgetPaintable::new(Some(&window));
        let snapshot = gtk::Snapshot::new();
        paintable.snapshot(&snapshot, f64::from(width), f64::from(height));

        let result = window
            .native()
            .and_then(|native| native.renderer())
            .zip(snapshot.to_node())
            .map(|(renderer, node)| renderer.render_texture(&node, None))
            .ok_or_else(|| "the window has no renderer yet".to_owned())
            .and_then(|texture| texture.save_to_png(&target).map_err(|e| e.to_string()));

        match result {
            Ok(()) => println!("{}", target.display()),
            Err(error) => eprintln!("grind-sheet-gtk: --render-to: {error}"),
        }
        window.close();
    });
}

#[cfg(test)]
mod chrome_tests {
    use super::*;

    /// Every `win.` action a menu model names, found by walking it — no display needed, since
    /// `gio::Menu` is GIO, which is why the menus are built by functions returning one.
    fn targets(model: &gio::MenuModel) -> Vec<String> {
        let mut found = Vec::new();
        for item in 0..model.n_items() {
            if let Some(action) = model
                .item_attribute_value(item, gio::MENU_ATTRIBUTE_ACTION, None)
                .and_then(|value| value.get::<String>())
            {
                found.push(action);
            }
            for link in [gio::MENU_LINK_SECTION, gio::MENU_LINK_SUBMENU] {
                if let Some(child) = model.item_link(item, link) {
                    found.extend(targets(&child));
                }
            }
        }
        found
    }

    /// Every action this window answers to: the verbs, plus the four readings `wire` builds
    /// by hand.
    fn every_action() -> Vec<&'static str> {
        actions()
            .into_iter()
            .map(|verb| verb.name)
            .chain(READINGS.iter().map(|(name, _, _)| *name))
            .chain(TARGETED)
            .collect()
    }

    /// The cell menu's spelling sections name the grid's own `spell.` actions, one section per
    /// word, and the language submenu offers every dictionary and both ends.
    #[test]
    fn the_spelling_menus_reach_the_grids_actions_and_every_dictionary() {
        let model = grid::spelling_menu_model(&[
            ("recieve".to_owned(), vec!["receive".to_owned()]),
            ("teh".to_owned(), Vec::new()),
        ]);
        assert_eq!(model.n_items(), 2, "one section per word");
        assert_eq!(
            targets(&model.upcast()),
            [
                "spell.correct",
                "spell.ignore",
                "spell.add",
                "spell.none",
                "spell.ignore",
                "spell.add"
            ]
        );
        let languages = chrome::spelling_menu_model();
        assert_eq!(
            languages.n_items() as usize,
            grind_spell::Setting::ALL.len()
        );
        assert!(
            targets(&languages.upcast())
                .iter()
                .all(|action| action == "win.spell-language")
        );
    }

    /// A menu item naming an action that does not exist is silent — GTK greys it, so it looks
    /// merely unavailable. This says so, over every menu in the window.
    #[test]
    fn every_menu_item_names_an_action_the_window_has() {
        let known = every_action();
        for (where_, model) in [
            ("the primary menu", primary_menu()),
            ("the View menu", chrome::view_menu_model()),
            ("the sheet tab menu", chrome::tab_menu_model()),
            ("the cell menu", grid::cell_menu_model()),
        ] {
            for action in targets(&model.upcast()) {
                let name = action.strip_prefix("win.").unwrap_or(&action);
                assert!(
                    known.contains(&name),
                    "{where_} points at win.{name}, which no verb declares"
                );
            }
        }
    }

    /// The surfaces divide the verbs rather than repeat them; the two exceptions mean different
    /// things where they are asked from, and are listed.
    #[test]
    fn a_verb_lives_in_one_menu() {
        let mut seen: Vec<(String, &str)> = Vec::new();
        for (where_, model) in [
            ("the primary menu", primary_menu()),
            ("the View menu", chrome::view_menu_model()),
            ("the sheet tab menu", chrome::tab_menu_model()),
            ("the cell menu", grid::cell_menu_model()),
        ] {
            for action in targets(&model.upcast()) {
                // `names` is the one verb that is both: naming *this range* from the cells,
                // and managing every name from the document menu.
                if action == "win.names" {
                    continue;
                }
                if let Some((_, first)) = seen.iter().find(|(seen, _)| *seen == action) {
                    // A radio group is one action under several targets: one verb, one menu.
                    let targeted = TARGETED.iter().any(|t| action == format!("win.{t}"));
                    if targeted && *first == where_ {
                        continue;
                    }
                    panic!("{action} is in {first} and again in {where_}");
                }
                seen.push((action, where_));
            }
        }
    }

    /// **The growth rule, made mechanical**: every verb is in the palette, which is built from the
    /// table `wire` is, so a feature cannot be added without becoming findable. The one exception
    /// is the command that opens the palette.
    #[test]
    fn every_verb_is_in_the_palette_or_is_the_palette() {
        let listed: Vec<&str> = palette_rows().iter().map(|row| row.name).collect();
        for name in every_action() {
            if TARGETED.contains(&name) {
                assert!(
                    !listed.contains(&name),
                    "{name} needs a target the palette cannot give"
                );
                continue;
            }
            if name == "palette" {
                assert!(
                    !listed.contains(&name),
                    "the palette must not offer to open itself"
                );
                continue;
            }
            assert!(
                listed.contains(&name),
                "win.{name} is a verb this window has and the palette does not offer — \
                 give it a title in `actions()`, or say here why it has none"
            );
        }
    }

    /// One action, one row. Two verbs sharing a name is one shadowing the other at
    /// `add_action`, and the loser simply never runs.
    #[test]
    fn no_two_verbs_share_a_name() {
        let mut names = every_action();
        let count = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), count, "a duplicate action name");
    }

    /// A reading is wired by hand, so it must not also be in `actions()` — `add_action` would
    /// replace the stateful one with a plain verb and the check marks would stop working.
    #[test]
    fn a_reading_is_never_also_a_plain_verb() {
        let verbs: Vec<&str> = actions().into_iter().map(|verb| verb.name).collect();
        for (name, _, _) in READINGS {
            assert!(
                !verbs.contains(&name),
                "{name} is both a reading and a verb"
            );
        }
    }

    /// Every verb a person can find is one they can read. A title with no group would sort
    /// into a palette section that does not exist.
    #[test]
    fn every_findable_verb_says_what_it_is_and_what_it_is_about() {
        for verb in actions() {
            if verb.title.is_empty() {
                continue;
            }
            assert!(!verb.group.is_empty(), "{} has no group", verb.name);
            // Sentence case with the shell's own capitals, never a trailing full stop: these
            // are labels, and a label that ends in a stop reads as a sentence cut short.
            assert!(
                !verb.title.ends_with('.'),
                "{}'s title ends in a stop: {:?}",
                verb.name,
                verb.title
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use grind_sheet::{Calculation, Pos};

    /// The handoff is decided from the bytes: a Writer document is the word processor's, a
    /// spreadsheet stays, and a Word document or Markdown — which say nothing in their bytes — go
    /// by name.
    #[test]
    fn a_text_document_is_known_by_its_bytes() {
        let data = |path: &str| {
            std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(path)).unwrap()
        };
        let path = std::path::Path::new;
        let text = data("../text/tests/data/numbered-list.fodt");
        assert!(
            super::is_text(path("renamed.fods"), &text),
            "bytes, not the name"
        );
        let sheet = data("../sheet/tests/data/kb/fizzbuzz.fods");
        assert!(!super::is_text(path("fizzbuzz.fods"), &sheet));
        assert!(super::is_text(path("letter.DOCX"), b"PK\x03\x04"));
        assert!(super::is_text(path("notes.md"), b"# Notes"));
        assert!(!super::is_text(path("data.csv"), b"a,b"));
    }

    fn calc(formula: &str, functions: &[&str]) -> Calculation {
        Calculation {
            sheet: 0,
            sheet_name: "Sheet1".to_owned(),
            pos: Pos::new(0, 0),
            formula: formula.to_owned(),
            value: "1".to_owned(),
            functions: functions.iter().map(|f| (*f).to_owned()).collect(),
        }
    }

    #[test]
    fn the_summary_counts_the_cells_and_names_the_functions() {
        let found = [
            calc("=SUM(A1:A9)", &["SUM"]),
            calc("=ROUND(SUM(B1:B9);2)", &["ROUND", "SUM"]),
        ];
        assert_eq!(super::tally(&found), "2 calculations — SUM ×2, ROUND ×1");
        // Arithmetic is a calculation with no functions in it, and one is not "1 calculations".
        assert_eq!(super::tally(&found[..1]), "1 calculation — SUM ×1");
        assert_eq!(super::tally(&[calc("=A1/2", &[])]), "1 calculation");
        assert!(super::tally(&[]).starts_with("Nothing here is calculated"));
    }

    #[test]
    fn a_row_reads_in_the_friendly_spelling_when_there_is_one() {
        assert_eq!(
            super::headline(&calc("=ROUND(A1;2)", &["ROUND"])),
            "Round(Value: A1, Digits: 2)"
        );
        // A formula that will not parse is shown exactly as it is stored.
        assert_eq!(super::headline(&calc("=SUM(", &[])), "=SUM(");
    }

    /// `doc/flat-first.md`, at the one keystroke that matters: the name a Save As dialog offers
    /// for a document that has never been saved. A document that already has a path keeps it,
    /// so nothing is ever converted behind somebody's back.
    #[test]
    fn an_unnamed_document_is_offered_the_flat_extension() {
        use std::path::Path;
        assert_eq!(super::save_name(None), "Untitled.fods");
        assert_eq!(
            super::save_name(Some(Path::new("/tmp/book.ods"))),
            "book.ods"
        );
        assert_eq!(
            super::save_name(Some(Path::new("/tmp/book.fods"))),
            "book.fods"
        );
    }
}

/// Whether a file is the word processor's, decided from its bytes where they can say —
/// `grind_core::kind`, before any parsing — and from its name where they cannot: a Word document
/// (a zip that is not ODF) and Markdown (plain text, with no signature), both of which
/// `grind-text-gtk` opens as a new document. The twin of that window's `is_spreadsheet`.
fn is_text(path: &Path, bytes: &[u8]) -> bool {
    match grind_core::kind::kind(bytes) {
        Some(found) => found == DocumentKind::Text,
        None => path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| {
                ["docx", "docm", "dotx", "md", "markdown"]
                    .contains(&ext.to_ascii_lowercase().as_str())
            }),
    }
}
