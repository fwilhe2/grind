// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `sheet-gtk` — the GNOME shell, and phase 9's first one.
//!
//! A renderer and an event forwarder that owns nothing (doc/plan.md rule 1). All state is
//! in `sheet-core`'s [`App`]; this crate holds a window, a grid that draws whatever
//! [`App::get_viewport`] returns, and the keys and editing around it. If a field shows up
//! here that is not a presentation concern, the core is missing something.
//!
//! `doc/gtk-shell.md` is the plan and the running record of what is built. Milestones 1, 3
//! and 4 are here: a document opens, draws, navigates, and can be edited and saved.
//!
//! **The core pushes; this never polls** (rule 3). `App`'s observer is a channel sender —
//! it must be `Send`, and a GTK widget is not — and one local future drains it, coalescing
//! a burst of changes into a single refresh.

mod chrome;
mod formatting;
mod formula_ux;
mod geom;
mod grid;
mod keymap;
mod state;
mod theme;

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::rc::Rc;
use std::sync::Arc;

use libadwaita as adw;
use libadwaita::gtk;
use libadwaita::prelude::*;

use gtk::{gio, glib};
use sheet_core::{App, Form, Observer, a1};

use grid::{Grid, Notice};

/// The reverse-DNS identity GNOME keys a window, its icon and its settings on.
const APP_ID: &str = "io.github.fwilhe2.Sheet";

fn main() -> ExitCode {
    let app = Arc::new(App::new());

    // The document is opened before the toolkit starts, so a bad path is a shell error
    // with an exit code rather than an empty window with a dialog in front of it. Like
    // every other shell here, a missing file is an error and not a new document.
    let mut args = std::env::args_os().skip(1);
    let path: Option<PathBuf> = args.next().map(PathBuf::from);
    // `--render-to <png>` draws one frame, writes it and exits. Not a user feature: it is
    // how a machine checks that the grid still draws, since a custom-drawn widget has no
    // other assertable output and `editor`'s §5 rule is to exercise every boundary with a
    // program that runs where the UI cannot.
    let render_to: Option<PathBuf> = match args.next() {
        Some(flag) if flag == "--render-to" => Some(PathBuf::from(
            args.next().unwrap_or_else(|| "grid.png".into()),
        )),
        _ => None,
    };
    if let Some(path) = &path
        && let Err(error) = app.open_file(path)
    {
        eprintln!("sheet-gtk: {}: {error}", path.display());
        return ExitCode::FAILURE;
    }

    let application = adw::Application::builder().application_id(APP_ID).build();
    application.connect_activate(move |application| {
        theme::install();
        let ui = Ui::build(application, &app, path.clone());
        ui.window.present();
        if let Some(target) = render_to.clone() {
            render_once(&ui.window, target);
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
    title: adw::WindowTitle,
    toasts: adw::ToastOverlay,
    banner: adw::Banner,
    tabs: Rc<chrome::Tabs>,
    strip: Rc<formatting::Strip>,
    formula_bar: Rc<chrome::FormulaBar>,
    undo: gtk::Button,
    redo: gtk::Button,
    path: RefCell<Option<PathBuf>>,
    dirty: Cell<bool>,
    /// Set by a load, and consumed by the change it is about to cause — opening a document
    /// notifies like any other change, and it must not leave the new one marked as modified.
    loading: Cell<bool>,
    /// `editor`'s latch: a close that is waiting on a save must not ask again.
    closing: Cell<bool>,
}

impl Ui {
    fn build(application: &adw::Application, app: &Arc<App>, path: Option<PathBuf>) -> Rc<Self> {
        let grid = Grid::new(app.clone());
        let scroller = gtk::ScrolledWindow::builder()
            .hexpand(true)
            .vexpand(true)
            .child(&grid)
            .build();

        let toasts = adw::ToastOverlay::new();
        toasts.set_child(Some(&scroller));

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

        let menu = gtk::MenuButton::builder()
            .icon_name("open-menu-symbolic")
            .menu_model(&primary_menu())
            .tooltip_text("Main Menu")
            .build();
        header.pack_end(&menu);

        let banner = adw::Banner::new("");
        let add_sheet = gtk::Button::from_icon_name("list-add-symbolic");
        add_sheet.set_tooltip_text(Some("Add Sheet"));
        add_sheet.set_action_name(Some("win.sheet-add"));
        add_sheet.add_css_class("flat");
        let tabs = chrome::Tabs::new(&grid, app, &add_sheet);
        let strip = formatting::strip(&grid, app);

        let view = adw::ToolbarView::builder().content(&toasts).build();
        view.add_top_bar(&header);
        view.add_top_bar(&strip.widget);
        let formula_bar = chrome::formula_bar(&grid, app, true);
        view.add_top_bar(&formula_bar.widget);
        view.add_top_bar(&banner);
        view.add_bottom_bar(&tabs.widget);
        view.add_bottom_bar(&chrome::status_bar(&grid, app));

        let window = adw::ApplicationWindow::builder()
            .application(application)
            .default_width(1100)
            .default_height(700)
            .content(&view)
            .build();
        // The grid is what the keys are for, so it starts focused rather than waiting for a
        // click to make the arrow keys work. Two traits spell `set_focus`; this is the one.
        gtk::prelude::GtkWindowExt::set_focus(&window, Some(&grid));

        let ui = Rc::new(Self {
            app: app.clone(),
            window,
            grid,
            title,
            toasts,
            banner,
            tabs,
            strip,
            formula_bar,
            undo,
            redo,
            path: RefCell::new(path),
            dirty: Cell::new(false),
            loading: Cell::new(false),
            closing: Cell::new(false),
        });
        ui.wire(application);
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
        glib::spawn_future_local(glib::clone!(
            #[strong(rename_to = ui)]
            self,
            async move {
                while receiver.recv().await.is_ok() {
                    // Drain: N changes are one repaint, which is what keeps a recalculation
                    // of a thousand cells from queueing a thousand frames.
                    while receiver.try_recv().is_ok() {}
                    ui.dirty.set(!ui.loading.replace(false));
                    ui.refresh();
                    ui.grid.invalidate();
                }
            }
        ));

        self.grid.connect_notice(glib::clone!(
            #[strong(rename_to = ui)]
            self,
            move |notice| ui.notice(notice)
        ));

        self.banner.connect_button_clicked(glib::clone!(
            #[strong(rename_to = ui)]
            self,
            move |_| ui.recalculate()
        ));

        for (name, accels, handler) in actions() {
            let action = gio::SimpleAction::new(name, None);
            action.connect_activate(glib::clone!(
                #[strong(rename_to = ui)]
                self,
                move |_, _| handler(&ui)
            ));
            self.window.add_action(&action);
            if !accels.is_empty() {
                application.set_accels_for_action(&format!("win.{name}"), accels);
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

        self.window.connect_close_request(glib::clone!(
            #[strong(rename_to = ui)]
            self,
            move |_| ui.confirm_close()
        ));
    }

    /// Everything derived from the document, in one place, run after every change.
    fn refresh(self: &Rc<Self>) {
        self.title.set_title(&document_name(self.path.borrow().as_deref()));
        self.title.set_subtitle(match self.dirty.get() {
            true => "Unsaved changes",
            false => "",
        });
        self.undo.set_sensitive(self.app.can_undo());
        self.redo.set_sensitive(self.app.can_redo());
        self.tabs.refresh();
        // Undo, redo and a load all change the cell under an unmoved selection, which is
        // what the format strip is showing.
        self.strip.refresh();
    }

    fn toast(&self, text: &str) {
        self.toasts.add_toast(adw::Toast::new(text));
    }

    /// A toast with an Undo button — the HIG pattern for anything destructive that is
    /// cheaper to do and take back than to confirm first.
    fn undoable_toast(self: &Rc<Self>, text: &str) {
        let toast = adw::Toast::builder().title(text).button_label("Undo").build();
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
            // A banner rather than a toast: this is a *state* the document is in, not an
            // event that happened, and it stays true until something recalculates.
            Notice::RecalcSkipped(spoiled) => {
                self.banner.set_title(&format!(
                    "{spoiled} formula cell(s) use functions this build does not have — \
                     recalculating would replace their saved values"
                ));
                self.banner.set_button_label(Some("Recalculate Anyway"));
                self.banner.set_revealed(true);
            }
        }
    }

    fn recalculate(self: &Rc<Self>) {
        self.banner.set_revealed(false);
        match self.app.recalc() {
            Ok(recalc) if recalc.spoiled > 0 => {
                self.undoable_toast(&format!("{} cell(s) became errors", recalc.spoiled))
            }
            Ok(recalc) if recalc.changed > 0 => {
                self.toast(&format!("{} cell(s) recalculated", recalc.changed))
            }
            Ok(_) => self.toast("Already up to date"),
            Err(error) => self.toast(&error.to_string()),
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
                self.dirty.set(false);
                self.refresh();
                self.toast("Saved");
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
            .filters(&spreadsheet_filters())
            .initial_name(document_name(self.path.borrow().as_deref()))
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
        self.loading.set(true);
        match self.app.open_file(path) {
            Ok(()) => {
                *self.path.borrow_mut() = Some(path.to_owned());
                self.grid.set_sheet(0);
                remember_recent(path);
            }
            Err(error) => {
                self.loading.set(false);
                self.toast(&format!("Could not open: {error}"));
            }
        }
    }

    fn new_document(self: &Rc<Self>) {
        self.confirm_discard(glib::clone!(
            #[strong(rename_to = ui)]
            self,
            move || {
                // There is no `App::reset`, and there does not need to be: an empty document
                // written and read back is one, through the same path a file takes.
                let Ok(bytes) = sheet_core::write_bytes(&sheet_core::Document::default(), Form::Flat)
                else {
                    return;
                };
                ui.loading.set(true);
                if ui.app.open_bytes("untitled.fods", &bytes).is_ok() {
                    *ui.path.borrow_mut() = None;
                    ui.grid.set_sheet(0);
                }
            }
        ));
    }

    // --- names ---

    /// The names dialog: every defined name, editable in place, plus a row to add one.
    ///
    /// A name is document-level and there is no other window to hang it off, so this is a
    /// list rather than a property panel. Each row is an `adw::EntryRow` over the name's
    /// definition, which makes editing one the same gesture as reading it; the core does
    /// every refusal, and its message is the toast.
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
                            None => list.append(&name_row(&app, &name, &expression, &say, &sync_empty)),
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

    // --- sheets ---

    fn add_sheet(self: &Rc<Self>) {
        // The first free `SheetN`, which is what every spreadsheet offers and nobody has to
        // think about.
        let taken: Vec<String> = (0..self.app.sheet_count())
            .filter_map(|i| self.app.sheet_name(i).ok())
            .collect();
        let name = (1..)
            .map(|n| format!("Sheet{n}"))
            .find(|name| !taken.iter().any(|t| t.eq_ignore_ascii_case(name)))
            .expect("there is always a free number");
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
                    // A rename does not rewrite the formulas that name the old sheet — they
                    // go stale, which `App::stale` counts and a recalculation turns into
                    // errors. Saying so here is what keeps that from being a surprise.
                    if let Err(error) = ui.app.rename_sheet(sheet, entry.text().trim()) {
                        ui.toast(&error.to_string());
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

    /// M9's shortcuts dialog. Built from the same accelerator table the window wires up, so
    /// it cannot list a binding the keyboard does not actually have — plus the grid's own
    /// vocabulary (`keymap.rs`), which has no `GAction` to read a name from.
    fn shortcuts(&self) {
        let window = gtk::ShortcutsWindow::builder()
            .transient_for(&self.window)
            .modal(true)
            .build();

        let group = gtk::ShortcutsGroup::builder().title("General").build();
        let named = [
            ("new", "New"),
            ("open", "Open"),
            ("save", "Save"),
            ("save-as", "Save As"),
            ("undo", "Undo"),
            ("redo", "Redo"),
            ("recalc", "Recalculate Now"),
            ("explain-formula", "Explain Formula"),
            ("zoom-in", "Zoom In"),
            ("zoom-out", "Zoom Out"),
            ("zoom-reset", "Normal Size"),
        ];
        for (action, title) in named {
            for (name, accels, _) in actions() {
                if name != action || accels.is_empty() {
                    continue;
                }
                group.add_shortcut(
                    &gtk::ShortcutsShortcut::builder()
                        .title(title)
                        .accelerator(accels.join(" "))
                        .build(),
                );
            }
        }
        let navigation = gtk::ShortcutsGroup::builder().title("Navigation & Editing").build();
        for (accelerator, title) in [
            ("Left Right Up Down", "Move selection"),
            ("<Control>Left <Control>Right <Control>Up <Control>Down", "Jump to data edge"),
            ("<Shift>Left <Shift>Right <Shift>Up <Shift>Down", "Extend selection"),
            ("<Control>a", "Select all"),
            ("Tab ISO_Left_Tab", "Move within a row"),
            ("Return <Shift>Return", "Move within a column"),
            ("Delete BackSpace", "Clear selection"),
            ("F2", "Edit cell"),
            ("F4", "Cycle $ in a reference"),
            ("Escape", "Cancel edit"),
            ("<Control>c <Control>x <Control>v", "Copy, cut, paste"),
        ] {
            navigation.add_shortcut(
                &gtk::ShortcutsShortcut::builder()
                    .title(title)
                    .accelerator(accelerator)
                    .build(),
            );
        }

        let section = gtk::ShortcutsSection::builder().section_name("main").build();
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
            .website("https://github.com/fwilhe2/sheet")
            .license_type(gtk::License::Agpl30)
            .comments("An ODF-native spreadsheet.")
            .debug_info(sheet_core::build_info::describe(
                "sheet-gtk",
                env!("CARGO_PKG_VERSION"),
            ))
            .build();
        about.present(Some(&self.window));
    }
}

/// The window's actions, their accelerators, and what they do. One table, so the menu, the
/// header bar and the keyboard cannot drift apart.
type Handler = fn(&Rc<Ui>);

/// One name, as a row: its definition editable in place, and a button that removes it.
///
/// `set_name` over the same name replaces it, so applying an edit is a definition and needs
/// no separate "rename" path — the name itself is the row's title and does not change.
fn name_row(
    app: &Arc<App>,
    name: &str,
    expression: &str,
    say: &Rc<dyn Fn(&str)>,
    sync_empty: &Rc<dyn Fn()>,
) -> adw::EntryRow {
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
        #[to_owned]
        name,
        move |row| {
            if let Err(error) = define(&app, &name, &row.text()) {
                say(&error);
            }
        }
    ));

    let delete = gtk::Button::from_icon_name("user-trash-symbolic");
    delete.set_tooltip_text(Some("Delete"));
    delete.set_valign(gtk::Align::Center);
    delete.add_css_class("flat");
    delete.connect_clicked(glib::clone!(
        #[strong]
        app,
        #[strong]
        sync_empty,
        #[weak]
        row,
        #[to_owned]
        name,
        move |_| {
            // A formula that mentions a deleted name goes stale rather than being rewritten
            // — `App::clear_name`'s documented answer, and the banner counts it.
            app.clear_name(&name);
            if let Some(list) = row.parent().and_downcast::<gtk::ListBox>() {
                list.remove(&row);
            }
            sync_empty();
        }
    ));
    row.add_suffix(&delete);
    row
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

fn actions() -> Vec<(&'static str, &'static [&'static str], Handler)> {
    vec![
        ("new", &["<Control>n"][..], (|ui: &Rc<Ui>| ui.new_document()) as Handler),
        ("open", &["<Control>o"][..], |ui| ui.open()),
        ("save", &["<Control>s"][..], |ui| ui.save()),
        ("save-as", &["<Control><Shift>s"][..], |ui| ui.save_as()),
        ("undo", &["<Control>z"][..], |ui| {
            ui.app.undo();
        }),
        ("redo", &["<Control><Shift>z", "<Control>y"][..], |ui| {
            ui.app.redo();
        }),
        ("recalc", &["F9"][..], |ui| ui.recalculate()),
        // Both spellings of the key, because a keyboard's `+` is `plus` with Shift and
        // `equal` without, and a user pressing Ctrl and the key next to Backspace means one
        // thing by it either way.
        ("zoom-in", &["<Control>plus", "<Control>equal", "<Control>KP_Add"][..], |ui| {
            ui.grid.set_zoom(ui.grid.zoom() * grid::ZOOM_STEP)
        }),
        ("zoom-out", &["<Control>minus", "<Control>KP_Subtract"][..], |ui| {
            ui.grid.set_zoom(ui.grid.zoom() / grid::ZOOM_STEP)
        }),
        ("zoom-reset", &["<Control>0", "<Control>KP_0"][..], |ui| ui.grid.set_zoom(1.0)),
        ("autofit-all", &[][..], |ui| ui.grid.autofit_all()),
        ("names", &[][..], |ui| ui.manage_names()),
        ("explain-formula", &["<Control><Shift>e"][..], |ui| ui.formula_bar.explain.popup()),
        ("sheet-add", &[][..], |ui| ui.add_sheet()),
        ("sheet-rename", &[][..], |ui| ui.rename_sheet()),
        ("sheet-delete", &[][..], |ui| ui.delete_sheet()),
        ("shortcuts", &["<Control>question"][..], |ui| ui.shortcuts()),
        ("about", &[][..], |ui| ui.about()),
    ]
}

fn primary_menu() -> gio::Menu {
    let menu = gio::Menu::new();
    let files = gio::Menu::new();
    files.append(Some("New"), Some("win.new"));
    files.append(Some("Open…"), Some("win.open"));
    files.append(Some("Save"), Some("win.save"));
    files.append(Some("Save As…"), Some("win.save-as"));
    menu.append_section(None, &files);

    let sheets = gio::Menu::new();
    sheets.append(Some("Add Sheet"), Some("win.sheet-add"));
    sheets.append(Some("Rename Sheet…"), Some("win.sheet-rename"));
    sheets.append(Some("Delete Sheet"), Some("win.sheet-delete"));
    menu.append_section(None, &sheets);

    let view = gio::Menu::new();
    view.append(Some("Zoom In"), Some("win.zoom-in"));
    view.append(Some("Zoom Out"), Some("win.zoom-out"));
    view.append(Some("Normal Size"), Some("win.zoom-reset"));
    view.append(Some("Resize All Cells to Fit"), Some("win.autofit-all"));
    view.append(Some("Friendly Formulas"), Some("win.friendly-formulas"));
    menu.append_section(None, &view);

    let rest = gio::Menu::new();
    rest.append(Some("Names…"), Some("win.names"));
    rest.append(Some("Explain Formula"), Some("win.explain-formula"));
    rest.append(Some("Recalculate Now"), Some("win.recalc"));
    rest.append(Some("Keyboard Shortcuts"), Some("win.shortcuts"));
    rest.append(Some("About Sheet"), Some("win.about"));
    menu.append_section(None, &rest);
    menu
}

/// The file filter, in the words a user knows the format by. Nothing user-facing names
/// another program (`CONTRIBUTING.md`).
///
/// One filter, both extensions: packaged and flat are the same document to everyone but the
/// writer, so a dialog that makes the user pick between them is asking a question that has
/// no answer at the point it is asked.
fn spreadsheet_filters() -> gio::ListStore {
    let filter = gtk::FileFilter::new();
    filter.set_name(Some("OpenDocument Spreadsheet"));
    filter.add_pattern("*.ods");
    filter.add_pattern("*.fods");

    let filters = gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&filter);
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

/// The document's name, or what an unsaved one is called until it has one.
fn document_name(path: Option<&Path>) -> String {
    path.and_then(|p| p.file_name()).map_or_else(
        || "Untitled".to_owned(),
        |name| name.to_string_lossy().into_owned(),
    )
}

/// Draw one frame to a PNG and quit — the smoke path behind `--render-to`.
///
/// A frame late enough to be real: the window has to be mapped and allocated before the
/// grid knows how many cells fit, so this waits for a beat rather than rendering an
/// unallocated widget and writing a blank image.
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
            Err(error) => eprintln!("sheet-gtk: --render-to: {error}"),
        }
        window.close();
    });
}
