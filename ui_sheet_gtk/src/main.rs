// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `grind-sheet-gtk` — the GNOME shell, and phase 9's first one.
//!
//! A renderer and an event forwarder that owns nothing (doc/plan.md rule 1). All state is
//! in `grind-sheet`'s [`App`]; this crate holds a window, a grid that draws whatever
//! [`App::get_viewport`] returns, and the keys and editing around it. If a field shows up
//! here that is not a presentation concern, the core is missing something.
//!
//! `doc/sheet-shell.md` is the plan and the running record of what is built. Milestones 1, 3
//! and 4 are here: a document opens, draws, navigates, and can be edited and saved.
//!
//! **The core pushes; this never polls** (rule 3). `App`'s observer is a channel sender —
//! it must be `Send`, and a GTK widget is not — and one local future drains it, coalescing
//! a burst of changes into a single refresh.

mod chart;
mod chrome;
mod code;
mod filter_ui;
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

use grind_sheet::{App, DocumentKind, Form, Observer, a1};
use gtk::{gio, glib};

use grid::{Grid, Notice};

/// The reverse-DNS identity GNOME keys a window, its icon and its settings on.
const APP_ID: &str = "io.github.fwilhe2.Sheet";

/// What this shell edits. The one input `save_name` needs beyond the form.
const KIND: DocumentKind = DocumentKind::Spreadsheet;

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
    let mut render_to: Option<PathBuf> = None;
    // `--overlay names|roles|all` turns `doc/view-modes.md`'s overlays on for that frame,
    // so the modes are assertable the same way the grid is: a refactor is proved one when
    // the PNG comes back byte-identical. Not a user feature either — the window's own
    // toggles are.
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
    if let Some(path) = &path
        && let Err(error) = app.open_file(path)
    {
        eprintln!("grind-sheet-gtk: {}: {error}", path.display());
        return ExitCode::FAILURE;
    }

    let application = adw::Application::builder().application_id(APP_ID).build();
    application.connect_activate(move |application| {
        theme::install();
        let ui = Ui::build(application, &app, path.clone());
        if overlays.any() {
            ui.grid.set_overlays(overlays);
        }
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
    /// The two pages of the window: the grid, and its projection (`doc/dsl.md` §6, D9).
    stack: gtk::Stack,
    source: gtk::TextView,
    /// Raised while the code view is being filled, so placing its cursor does not read back as
    /// the reader moving it — `formatting::Strip`'s latch, for the same reason.
    updating: Cell<bool>,
    /// The code view line the grid's selection is on, as last marked.
    ///
    /// The latch above is not enough on its own: GTK does not always deliver
    /// `notify::cursor-position` inside the call that caused it, so a handler that answers by
    /// touching the cursor is a loop the latch never sees the re-entry of. This makes the
    /// handler *idempotent* instead — the same line twice does nothing — which holds however
    /// the signal is scheduled.
    marked: Cell<Option<usize>>,
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

        // The code view, on the other page of a stack (`doc/dsl.md` §6.1). A stack rather than a
        // paned split: §6.2 is right that a split is what a person eventually wants, and it is
        // also a second viewport to keep in step. The correspondence is what pays for itself
        // first, and one page at a time carries it.
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

        let menu = gtk::MenuButton::builder()
            .icon_name("open-menu-symbolic")
            .menu_model(&primary_menu())
            .tooltip_text("Main Menu")
            .build();
        header.pack_end(&menu);

        // **Labelled, not icon-only.** There is no chart icon in the icon theme — the name
        // this button used to carry (`view-object-select-symbolic`) is not in Adwaita at all,
        // so it drew as the missing-image glyph. A word is better than a wrong picture, and
        // this is the one button in the header that names a whole feature rather than
        // repeating a menu item.
        let chart_content = adw::ButtonContent::builder()
            .label("Chart")
            .icon_name("insert-object-symbolic")
            .build();
        let insert_chart = gtk::Button::builder()
            .child(&chart_content)
            .tooltip_text("Insert a chart from the selected cells")
            .action_name("win.chart-insert")
            .build();
        header.pack_start(&insert_chart);

        let banner = adw::Banner::new("");
        let add_sheet = gtk::Button::from_icon_name("list-add-symbolic");
        add_sheet.set_tooltip_text(Some("Add Sheet"));
        add_sheet.set_action_name(Some("win.sheet-add"));
        add_sheet.add_css_class("flat");
        let tabs = chrome::Tabs::new(&grid, app, &add_sheet);
        let strip = formatting::strip(&grid, app);

        let view = adw::ToolbarView::builder().content(&toasts).build();
        view.add_top_bar(&header);
        // The strip is the Format page of the mode-switched tool row — Calculate and View
        // sit behind the same switch instead of only in the primary menu.
        view.add_top_bar(&chrome::tools(&grid, &strip.widget));
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
            stack,
            source,
            updating: Cell::new(false),
            marked: Cell::new(None),
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

        // `doc/view-modes.md`'s two overlays, stateful for the same reason: each is a way of
        // *looking* at the document rather than something done to it, so the toggle carries
        // its own on/off and turning it off leaves nothing behind. Neither writes a byte,
        // which is why they need no confirmation, no dirty flag and no undo entry.
        for (name, accels, names, roles) in [
            ("show-names", &["<Control><Shift>n"][..], true, false),
            ("show-roles", &["<Control><Shift>r"][..], false, true),
        ] {
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
            application.set_accels_for_action(&format!("win.{name}"), accels);
        }

        // `doc/dsl.md` §6, D9 — the document as its projection, on the other page of the
        // stack. Stateful like the two overlays above and for the same reason: it is a way of
        // *looking* at the document, it writes nothing, and the same item turns it off.
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
        application.set_accels_for_action("win.show-source", &["<Control><Shift>u"]);

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

    // --- the code view (doc/dsl.md §6, D9) ---

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

    /// The source's cursor moved: select the cell that line projects.
    ///
    /// A **sheet's own name** is checked first, because a `sheet` node anchors one and a bare
    /// name is also a perfectly good cell address — `Sheet1` parses as column `SHEET`, row 1, and
    /// answering a click on `sheet Sales {` with a jump to a cell nobody has ever used would be
    /// worse than doing nothing.
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
        self.title
            .set_title(&document_name(self.path.borrow().as_deref()));
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
            // A banner rather than a toast: this is a *state* the document is in, not an
            // event that happened, and it stays true until something recalculates.
            Notice::RecalcSkipped(spoiled) => {
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
        match self.app.recalc() {
            Ok(recalc) if recalc.spoiled > 0 => self.undoable_toast(&counted(
                recalc.spoiled,
                "cell became an error",
                "cells became errors",
            )),
            Ok(recalc) if recalc.changed > 0 => self.toast(&counted(
                recalc.changed,
                "cell recalculated",
                "cells recalculated",
            )),
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
            .initial_name(save_name(self.path.borrow().as_deref()))
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
                let Ok(bytes) =
                    grind_sheet::write_bytes(&grind_sheet::Document::default(), Form::Flat)
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

    /// The calculations dialog: every formula in the document, searchable, each row jumping
    /// to the cell it names.
    ///
    /// A spreadsheet shows results and hides the formulas behind them, so the one question a
    /// grid cannot answer is "what in here is computed, and out of what". Plain arithmetic is
    /// listed beside function calls — `=A1/2` is as much a calculation as `=SUM(A1:A9)` — and
    /// the search matches a function name as readily as an address, which is what makes it
    /// also the answer to "where is TODAY used".
    ///
    /// The list is rebuilt per keystroke rather than filtered in place: `App::calculations`
    /// walks the formulas already in memory, and a document with enough of them to notice
    /// has bigger problems.
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

    // --- charts ---

    /// The chart dialog — **one dialog, both jobs**: `None` inserts a chart, `Some(index)`
    /// edits the one already there. Every field means the same thing either way, so writing
    /// them twice would only be two places for them to drift apart.
    ///
    /// Inserting prefills from the current selection when it spans more than one row and more
    /// than one column: the first column becomes categories, the first row becomes each
    /// remaining column's own label, and each remaining column becomes a series — the shape a
    /// user selecting "Party" and a column of vote counts already has in mind. A smaller
    /// selection leaves the fields blank, the same as typing `grind sheet chart-add` with
    /// nothing pre-filled. Editing prefills from the chart itself.
    fn chart_dialog(self: &Rc<Self>, editing: Option<usize>) {
        let sheet = self.grid.sheet();
        let existing = editing.and_then(|index| self.app.charts(sheet).ok()?.get(index).cloned());
        if editing.is_some() && existing.is_none() {
            return;
        }

        let (kind_default, categories_default, series_defaults, x_axis, y_axis) = match &existing {
            Some(chart) => (
                match chart.kind {
                    grind_sheet::ChartKind::Bar => 0,
                    grind_sheet::ChartKind::Line => 1,
                    grind_sheet::ChartKind::Pie => 2,
                },
                chart.categories.clone().unwrap_or_default(),
                chart
                    .series
                    .iter()
                    .map(|s| match &s.label {
                        Some(label) => format!("{}={label}", s.values),
                        None => s.values.clone(),
                    })
                    .collect(),
                chart.x_axis.clone(),
                chart.y_axis.clone(),
            ),
            None => {
                let (categories, series) = self.selection_as_chart();
                (
                    0,
                    categories,
                    series,
                    grind_sheet::ChartAxis::default(),
                    grind_sheet::ChartAxis::default(),
                )
            }
        };
        let mut series_defaults = series_defaults;
        if series_defaults.is_empty() {
            series_defaults.push(String::new());
        }

        let kind = adw::ComboRow::builder()
            .title("Type")
            .model(&gtk::StringList::new(&["Bar", "Line", "Pie"]))
            .selected(kind_default)
            .build();
        let categories = adw::EntryRow::builder()
            .title("Categories (x axis), e.g. B3:B9")
            .text(&categories_default)
            .build();

        let series_list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .build();
        series_list.add_css_class("boxed-list");
        for text in &series_defaults {
            series_list.append(&series_row(text));
        }
        let add_series = gtk::Button::with_label("+ Add Series");
        add_series.set_halign(gtk::Align::Start);
        add_series.connect_clicked(glib::clone!(
            #[weak]
            series_list,
            move |_| series_list.append(&series_row(""))
        ));

        let x = axis_group("X axis (categories)", "e.g. Party", &x_axis);
        let y = axis_group("Y axis (values)", "e.g. Votes", &y_axis);
        let (weak_x, weak_y) = (x.weak(), y.weak());

        let type_list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .build();
        type_list.add_css_class("boxed-list");
        type_list.append(&kind);
        type_list.append(&categories);

        let apply = gtk::Button::with_label(match editing {
            Some(_) => "Apply",
            None => "Insert",
        });
        apply.add_css_class("suggested-action");
        apply.set_halign(gtk::Align::End);

        let content = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(12)
            .margin_top(12)
            .margin_bottom(12)
            .margin_start(12)
            .margin_end(12)
            .build();
        for widget in [
            type_list.upcast_ref::<gtk::Widget>(),
            series_list.upcast_ref(),
            add_series.upcast_ref(),
            x.group.upcast_ref(),
            y.group.upcast_ref(),
            apply.upcast_ref(),
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
        let dialog = adw::Dialog::builder()
            .title(match editing {
                Some(_) => "Edit Chart",
                None => "Insert Chart",
            })
            .content_width(460)
            .content_height(600)
            .child(&view)
            .build();

        apply.connect_clicked(glib::clone!(
            #[strong(rename_to = ui)]
            self,
            #[weak]
            dialog,
            #[weak]
            kind,
            #[weak]
            categories,
            #[weak]
            series_list,
            move |_| {
                let chart_kind = match kind.selected() {
                    1 => grind_sheet::ChartKind::Line,
                    2 => grind_sheet::ChartKind::Pie,
                    _ => grind_sheet::ChartKind::Bar,
                };
                let categories_text = categories.text();
                let categories_range =
                    (!categories_text.trim().is_empty()).then(|| categories_text.trim().to_owned());

                let mut series_texts = Vec::new();
                let mut row = series_list.first_child();
                while let Some(r) = row {
                    if let Some(entry) = r.downcast_ref::<adw::EntryRow>() {
                        let text = entry.text();
                        if !text.trim().is_empty() {
                            series_texts.push(text.trim().to_owned());
                        }
                    }
                    row = r.next_sibling();
                }
                if series_texts.is_empty() {
                    ui.toast("At least one series is required");
                    return;
                }
                let series: Vec<(String, Option<String>)> = series_texts
                    .iter()
                    .map(|s| match s.split_once('=') {
                        Some((values, label)) => (values.to_owned(), Some(label.to_owned())),
                        None => (s.clone(), None),
                    })
                    .collect();
                let series: Vec<(&str, Option<&str>)> = series
                    .iter()
                    .map(|(v, l)| (v.as_str(), l.as_deref()))
                    .collect();

                let (Some(x_axis), Some(y_axis)) = (weak_x.read(), weak_y.read()) else {
                    return;
                };

                let sheet = ui.grid.sheet();
                let result = match editing {
                    Some(index) => ui.app.edit_chart(
                        sheet,
                        index,
                        chart_kind,
                        categories_range.as_deref(),
                        &series,
                        x_axis,
                        y_axis,
                    ),
                    None => {
                        // Successive inserts land at slightly different spots, so they don't
                        // stack exactly on top of each other — a user repositions by dragging
                        // afterward either way.
                        let n = ui.app.charts(sheet).map(|c| c.len()).unwrap_or(0) as f64;
                        let at = grind_sheet::style::mm_length(20.0 + n * 5.0);
                        ui.app.add_chart(
                            sheet,
                            chart_kind,
                            categories_range.as_deref(),
                            &series,
                            &at,
                            &at,
                            "12cm",
                            "8cm",
                            x_axis,
                            y_axis,
                        )
                    }
                };
                match result {
                    Ok(()) => {
                        dialog.close();
                    }
                    Err(error) => ui.toast(&error.to_string()),
                }
            }
        ));

        dialog.present(Some(&self.window));
    }

    /// The current selection read as a chart's ranges: `(categories, series)`, both empty
    /// unless the selection spans more than one row *and* more than one column, which is the
    /// only shape where "the first column names the rest" is a safe guess.
    fn selection_as_chart(&self) -> (String, Vec<String>) {
        let (start, end) = self.grid.selection().rect();
        if end.row <= start.row || end.col <= start.col {
            return (String::new(), Vec::new());
        }
        let range = |row_from: u32, row_to: u32, col: u32| {
            format!(
                "{}:{}",
                a1::format(None, grind_sheet::Pos::new(row_from, col)),
                a1::format(None, grind_sheet::Pos::new(row_to, col))
            )
        };
        let categories = range(start.row + 1, end.row, start.col);
        let series = ((start.col + 1)..=end.col)
            .map(|col| {
                format!(
                    "{}={}",
                    range(start.row + 1, end.row, col),
                    a1::format(None, grind_sheet::Pos::new(start.row, col))
                )
            })
            .collect();
        (categories, series)
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
                    // A rename carries every reference with it (`doc/dsl.md` §6.5, D10) —
                    // formulas, named expressions, chart ranges — in one undo step. The toast
                    // says how many, because a document-wide edit a user did not see happen is
                    // one they cannot trust; Ctrl+Z takes all of it back.
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
            ("calculations", "Find Calculations"),
            ("filter", "Filter Rows"),
            ("zoom-in", "Zoom In"),
            ("zoom-out", "Zoom Out"),
            ("zoom-reset", "Normal Size"),
        ];
        // The two view modes are stateful actions rather than verbs, so they are not in
        // `actions()` and are listed here with the accelerators `wire` gives them.
        for (title, accelerator) in [
            ("Show Names", "<Control><Shift>n"),
            ("Show Roles", "<Control><Shift>r"),
            ("Show Source", "<Control><Shift>u"),
        ] {
            group.add_shortcut(
                &gtk::ShortcutsShortcut::builder()
                    .title(title)
                    .accelerator(accelerator)
                    .build(),
            );
        }
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

/// One axis' worth of the chart dialog: a title, and a switch for each of the two things an
/// axis can draw. Kept together with a [`WeakAxis::read`] that turns the widgets back into the
/// [`grind_sheet::ChartAxis`] the core takes, so what is shown and what is stored are never
/// assembled in two different places.
struct AxisGroup {
    group: adw::PreferencesGroup,
    label: adw::EntryRow,
    tick_labels: adw::SwitchRow,
    gridlines: adw::SwitchRow,
}

impl AxisGroup {
    /// The same three widgets, held weakly — what the Apply handler captures. A *strong*
    /// capture there is a reference cycle (dialog → button → closure → row → the dialog it
    /// is in), which is why every other widget that handler reads is `#[weak]` too.
    fn weak(&self) -> WeakAxis {
        WeakAxis {
            label: self.label.downgrade(),
            tick_labels: self.tick_labels.downgrade(),
            gridlines: self.gridlines.downgrade(),
        }
    }
}

struct WeakAxis {
    label: glib::WeakRef<adw::EntryRow>,
    tick_labels: glib::WeakRef<adw::SwitchRow>,
    gridlines: glib::WeakRef<adw::SwitchRow>,
}

impl WeakAxis {
    /// What the rows currently say, or `None` once the dialog they were in is gone.
    fn read(&self) -> Option<grind_sheet::ChartAxis> {
        let label = self.label.upgrade()?.text();
        let label = label.trim();
        Some(grind_sheet::ChartAxis {
            label: (!label.is_empty()).then(|| label.to_owned()),
            tick_labels: self.tick_labels.upgrade()?.is_active(),
            gridlines: self.gridlines.upgrade()?.is_active(),
        })
    }
}

fn axis_group(title: &str, hint: &str, axis: &grind_sheet::ChartAxis) -> AxisGroup {
    let group = adw::PreferencesGroup::builder().title(title).build();
    let label = adw::EntryRow::builder()
        .title(format!("Title, {hint}"))
        .text(axis.label.clone().unwrap_or_default())
        .build();
    let tick_labels = adw::SwitchRow::builder()
        .title("Tick labels")
        .subtitle("Name each value along this axis")
        .active(axis.tick_labels)
        .build();
    let gridlines = adw::SwitchRow::builder()
        .title("Gridlines")
        .active(axis.gridlines)
        .build();
    for row in [
        label.upcast_ref::<gtk::Widget>(),
        tick_labels.upcast_ref(),
        gridlines.upcast_ref(),
    ] {
        group.add(row);
    }
    AxisGroup {
        group,
        label,
        tick_labels,
        gridlines,
    }
}

/// One series in the chart dialog: `RANGE[=LABEL]`, the same vocabulary
/// `chart-add --series` already accepts, plus a button that removes the row.
fn series_row(text: &str) -> adw::EntryRow {
    let row = adw::EntryRow::builder()
        .title("Series (range or range=label-range)")
        .text(text)
        .build();
    let delete = gtk::Button::from_icon_name("user-trash-symbolic");
    delete.set_tooltip_text(Some("Remove"));
    delete.set_valign(gtk::Align::Center);
    delete.add_css_class("flat");
    delete.connect_clicked(glib::clone!(
        #[weak]
        row,
        move |_| {
            if let Some(list) = row.parent().and_downcast::<gtk::ListBox>() {
                list.remove(&row);
            }
        }
    ));
    row.add_suffix(&delete);
    row
}

/// What a calculation's row says first: the friendly rendering of its formula when there is
/// one, and the formula itself otherwise.
///
/// The friendly spelling because a list is read rather than edited — `Round(Value: A1;
/// Digits: 2)` answers "what does this cell do" quicker than `=ROUND(A1;2)` does, and the
/// address line underneath still names the cell to go and look at.
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

fn actions() -> Vec<(&'static str, &'static [&'static str], Handler)> {
    vec![
        (
            "new",
            &["<Control>n"][..],
            (|ui: &Rc<Ui>| ui.new_document()) as Handler,
        ),
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
        (
            "zoom-in",
            &["<Control>plus", "<Control>equal", "<Control>KP_Add"][..],
            |ui| ui.grid.set_zoom(ui.grid.zoom() * grid::ZOOM_STEP),
        ),
        (
            "zoom-out",
            &["<Control>minus", "<Control>KP_Subtract"][..],
            |ui| ui.grid.set_zoom(ui.grid.zoom() / grid::ZOOM_STEP),
        ),
        ("zoom-reset", &["<Control>0", "<Control>KP_0"][..], |ui| {
            ui.grid.set_zoom(1.0)
        }),
        ("autofit-all", &[][..], |ui| ui.grid.autofit_all()),
        // No accelerators: the grid's own key map already owns Ctrl+D, Ctrl+R and
        // Ctrl+Shift+C, and a second binding for the same key is how one shortcut ends up
        // doing two things. These exist so the tool strip can reach them.
        ("fill-down", &[][..], |ui| ui.grid.fill(keymap::Dir::Down)),
        ("fill-right", &[][..], |ui| ui.grid.fill(keymap::Dir::Right)),
        ("copy-value", &[][..], |ui| ui.grid.copy_value()),
        // Ctrl+Shift+L is the filter key both other spreadsheets use, and nothing in the
        // grid's own key map claims it.
        ("filter", &["<Control><Shift>l"][..], |ui| {
            ui.grid.toggle_filter()
        }),
        ("names", &[][..], |ui| ui.manage_names()),
        ("chart-insert", &[][..], |ui| ui.chart_dialog(None)),
        ("calculations", &["<Control><Shift>f"][..], |ui| {
            ui.explore_calculations()
        }),
        ("explain-formula", &["<Control><Shift>e"][..], |ui| {
            ui.formula_bar.explain.popup()
        }),
        ("sheet-add", &[][..], |ui| ui.add_sheet()),
        ("sheet-rename", &[][..], |ui| ui.rename_sheet()),
        ("sheet-delete", &[][..], |ui| ui.delete_sheet()),
        ("shortcuts", &["<Control>question"][..], |ui| ui.shortcuts()),
        ("about", &[][..], |ui| ui.about()),
    ]
}

/// The primary menu, slimmed to the HIG's idea of one: file and window-level items only.
/// The document tools that used to pile up here live in the tool strip's Calculate and
/// View pages (`chrome::tools`), where they are visible instead of buried.
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
    sheets.append(Some("Insert Chart…"), Some("win.chart-insert"));
    menu.append_section(None, &sheets);

    let rest = gio::Menu::new();
    rest.append(Some("Show Source"), Some("win.show-source"));
    rest.append(Some("Keyboard Shortcuts"), Some("win.shortcuts"));
    rest.append(Some("About Sheet"), Some("win.about"));
    menu.append_section(None, &rest);
    menu
}

/// The file filter, in the words a user knows the format by. Nothing user-facing names
/// another program (`CONTRIBUTING.md`).
///
/// One filter, all three extensions: packaged, flat and projected are the same document to
/// everyone but the writer, so an *open* dialog that makes the user pick between them is asking
/// a question that has no answer at the point it is asked.
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
    filters.append(&filter);
    filters
}

/// Saving is the other case: the question *does* have an answer there, and `doc/flat-first.md`
/// is that answer. Three filters rather than one, flat first, so the default selection writes
/// the form that diffs and choosing another is one deliberate click away.
///
/// The projection is last on purpose. It diffs better than flat XML does, but it is this
/// project's own spelling and nothing else reads it, whereas both ODF forms are a format other
/// software opens — so it is offered rather than defaulted to.
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

/// The document's name, or what an unsaved one is called until it has one.
fn document_name(path: Option<&Path>) -> String {
    path.and_then(|p| p.file_name()).map_or_else(
        || "Untitled".to_owned(),
        |name| name.to_string_lossy().into_owned(),
    )
}

/// What the Save As dialog puts in its name field for a document with no path yet.
///
/// `doc/flat-first.md`: an unnamed document gets the flat extension, so the one keystroke a
/// user is most likely *not* to change writes a file that diffs. A document that already has a
/// path keeps its own name and therefore its own form — this build never converts a document
/// behind somebody's back.
fn save_name(path: Option<&Path>) -> String {
    match path {
        Some(_) => document_name(path),
        None => format!("Untitled.{}", Form::Flat.extension(KIND)),
    }
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
            Err(error) => eprintln!("grind-sheet-gtk: --render-to: {error}"),
        }
        window.close();
    });
}

#[cfg(test)]
mod tests {
    use grind_sheet::{Calculation, Pos};

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
