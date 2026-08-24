// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `grind-text-gtk` — the GNOME shell for the word processor, and phase 10's S9.
//!
//! A renderer and an event forwarder that owns nothing (doc/plan.md rule 1). All state is in
//! `grind-text`'s [`App`]; this crate holds a window, a view that draws whatever
//! [`App::get_viewport`] and [`App::layout_block`] return, and the keys around it.
//!
//! **A second binary, not a mode.** `doc/suite.md` settled that: a `.desktop` file's
//! `MimeType=` is how a desktop associates an application with a file type, and one entry
//! claiming both spreadsheets and text documents is what makes "Open With" useless. So this
//! is `io.github.fwilhe2.Text` beside `io.github.fwilhe2.Sheet`, and opening a spreadsheet
//! here offers to hand it over rather than failing obscurely.
//!
//! **What this milestone is.** A *minimal* shell: it opens, draws, navigates by line, edits
//! at the caret, saves, and undoes. What it does not do is written down in `doc/text-shell.md`
//! rather than discovered — most of all that there is no selection, no styling UI and no
//! `grind-ui` crate yet, since `doc/suite.md` puts that extraction *on evidence* and one
//! minimal shell is not yet evidence of which seam to cut.

mod geom;
mod keymap;
mod metrics;
mod theme;
mod view;

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::rc::Rc;
use std::sync::Arc;

use libadwaita as adw;
use libadwaita::gtk;
use libadwaita::prelude::*;

use grind_core::{DocumentKind, Observer, kind};
use grind_text::{App, BlockKind, Form};
use gtk::{gio, glib};

use view::Doc;

/// The reverse-DNS identity GNOME keys a window, its icon and its settings on. Its own,
/// beside the spreadsheet's, for the reason at the top of this file.
const APP_ID: &str = "io.github.fwilhe2.Text";

/// What this shell edits. The one input `save_name` needs beyond the form.
const KIND: DocumentKind = DocumentKind::Text;

/// The sibling shell, launched by name when a spreadsheet is opened here.
const SHEET_APP: &str = "grind-sheet-gtk";

type Handler = fn(&Rc<Ui>);

fn main() -> ExitCode {
    let app = Arc::new(App::new());

    // One optional document, and `--render-to <png>`, which draws one frame, writes it and
    // exits — the same assertable output `grind-sheet-gtk` grew for the same reason: a
    // custom-drawn widget has no other one. Either order, because a flag that only works
    // last is a flag somebody will report as broken.
    let mut path: Option<PathBuf> = None;
    let mut render_to: Option<PathBuf> = None;
    let mut args = std::env::args_os().skip(1);
    while let Some(arg) = args.next() {
        match arg {
            arg if arg == "--render-to" => {
                render_to = Some(PathBuf::from(
                    args.next().unwrap_or_else(|| "document.png".into()),
                ));
            }
            arg if path.is_none() => path = Some(PathBuf::from(arg)),
            arg => {
                eprintln!(
                    "grind-text-gtk: unexpected argument {}",
                    arg.to_string_lossy()
                );
                return ExitCode::FAILURE;
            }
        }
    }

    // The document is opened before the toolkit starts, so a bad path is a shell error with
    // an exit code rather than an empty window with a dialog in front of it.
    if let Some(path) = &path
        && let Err(error) = app.open_file(path)
    {
        eprintln!("grind-text-gtk: {}: {error}", path.display());
        if is_spreadsheet(path) {
            eprintln!(
                "grind-text-gtk: that is a spreadsheet — try: {SHEET_APP} {}",
                path.display()
            );
        }
        return ExitCode::FAILURE;
    }
    // An empty document still needs a paragraph to type into.
    if path.is_none() {
        let _ = app.insert(0, BlockKind::Paragraph, "");
    }

    let application = adw::Application::builder().application_id(APP_ID).build();
    application.connect_activate(move |application| {
        let ui = Ui::build(application, &app, path.clone());
        ui.window.present();
        if let Some(target) = render_to.clone() {
            render_once(&ui.window, target);
        }
    });

    match application.run_with_args::<&str>(&[]) {
        code if code == glib::ExitCode::SUCCESS => ExitCode::SUCCESS,
        _ => ExitCode::FAILURE,
    }
}

/// The window and the pieces that have to be told when something changes.
///
/// Everything here is presentation. The document lives in `App`.
struct Ui {
    app: Arc<App>,
    window: adw::ApplicationWindow,
    doc: Doc,
    title: adw::WindowTitle,
    toasts: adw::ToastOverlay,
    banner: adw::Banner,
    status: gtk::Label,
    address: gtk::Entry,
    goto: gtk::MenuButton,
    undo: gtk::Button,
    redo: gtk::Button,
    path: RefCell<Option<PathBuf>>,
    /// The document a banner is offering to hand to the spreadsheet.
    handoff: RefCell<Option<PathBuf>>,
    dirty: Cell<bool>,
    /// Set by a load and consumed by the change it causes: opening notifies like any other
    /// change and must not leave the new document marked as modified.
    loading: Cell<bool>,
    /// A close waiting on a save must not ask again.
    closing: Cell<bool>,
}

impl Ui {
    fn build(application: &adw::Application, app: &Arc<App>, path: Option<PathBuf>) -> Rc<Self> {
        let doc = Doc::new(app.clone());
        let scroller = gtk::ScrolledWindow::builder()
            .hexpand(true)
            .vexpand(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(&doc)
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

        // The differentiator, in the smallest UI it fits in: a document address — `p12`,
        // `#intro`, `§2.1.3` — typed into a popover. `loc.rs` was designed for this, and
        // `§2.1` surviving edits above it is what no other word processor's UI offers.
        let address = gtk::Entry::builder()
            .placeholder_text("p12, #intro, \u{a7}2.1")
            .width_chars(18)
            .build();
        let popover = gtk::Popover::builder().child(&address).build();
        let goto = gtk::MenuButton::builder()
            .icon_name("find-location-symbolic")
            .tooltip_text("Go to Address")
            .popover(&popover)
            .build();
        header.pack_start(&goto);

        let menu = gtk::MenuButton::builder()
            .icon_name("open-menu-symbolic")
            .menu_model(&primary_menu())
            .tooltip_text("Main Menu")
            .build();
        header.pack_end(&menu);

        let banner = adw::Banner::new("");
        let status = gtk::Label::builder()
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build();
        status.add_css_class("dim-label");
        let status_bar = gtk::Box::builder()
            .margin_start(12)
            .margin_end(12)
            .margin_top(4)
            .margin_bottom(4)
            .build();
        status_bar.append(&status);

        let content = adw::ToolbarView::builder().content(&toasts).build();
        content.add_top_bar(&header);
        content.add_top_bar(&banner);
        content.add_bottom_bar(&status_bar);

        let window = adw::ApplicationWindow::builder()
            .application(application)
            .default_width(900)
            .default_height(760)
            .content(&content)
            .build();
        // The document is what the keys are for, so it starts focused rather than waiting
        // for a click to make the arrows work.
        gtk::prelude::GtkWindowExt::set_focus(&window, Some(&doc));

        let ui = Rc::new(Self {
            app: app.clone(),
            window,
            doc,
            title,
            toasts,
            banner,
            status,
            address,
            goto,
            undo,
            redo,
            path: RefCell::new(path),
            handoff: RefCell::new(None),
            dirty: Cell::new(false),
            loading: Cell::new(false),
            closing: Cell::new(false),
        });
        ui.wire(application);
        ui.refresh();
        ui
    }

    fn wire(self: &Rc<Self>, application: &adw::Application) {
        // The bridge. `Observer` is `Send + Sync` and a widget is neither, so what crosses is
        // a token; the local future does the reading, *after* the mutation that sent it has
        // released the lock (doc/plan.md rule 3).
        let (sender, receiver) = async_channel::unbounded::<()>();
        self.app.set_observer(Arc::new(Bridge(sender)));
        glib::spawn_future_local(glib::clone!(
            #[strong(rename_to = ui)]
            self,
            async move {
                while receiver.recv().await.is_ok() {
                    // Drain: N changes are one repaint, which is what keeps a replace-all
                    // over a long document from queueing a frame per paragraph.
                    while receiver.try_recv().is_ok() {}
                    ui.dirty.set(!ui.loading.replace(false));
                    ui.doc.invalidate();
                    ui.refresh();
                }
            }
        ));

        self.doc.connect_notice(glib::clone!(
            #[strong(rename_to = ui)]
            self,
            move |message| ui.toast(&message)
        ));
        self.doc.connect_moved(glib::clone!(
            #[strong(rename_to = ui)]
            self,
            move |_| ui.refresh()
        ));

        self.address.connect_activate(glib::clone!(
            #[strong(rename_to = ui)]
            self,
            move |entry| {
                match view::caret_of(&ui.app, entry.text().trim()) {
                    Ok(caret) => {
                        entry.set_text("");
                        ui.goto.popdown();
                        ui.doc.go_to(caret);
                        ui.doc.grab_focus();
                    }
                    Err(error) => ui.toast(&error),
                }
            }
        ));

        // The banner's one button: hand this document to the shell that does open it.
        self.banner.connect_button_clicked(glib::clone!(
            #[strong(rename_to = ui)]
            self,
            move |_| ui.hand_over()
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

        self.window.connect_close_request(glib::clone!(
            #[strong(rename_to = ui)]
            self,
            move |_| ui.confirm_close()
        ));
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

        let caret = self.doc.caret();
        let counts = self.app.counts();
        let here = match self
            .app
            .get_viewport(caret.block..caret.block + 1)
            .get(caret.block)
        {
            Some(block) => describe_kind(&block.kind, block.style.as_deref()),
            None => "empty".to_owned(),
        };
        self.status.set_text(&format!(
            "{}   {here}   {} words, {} blocks",
            grind_text::loc::format_offset(caret.block, caret.offset),
            counts.words,
            counts.blocks,
        ));
    }

    fn toast(&self, text: &str) {
        self.toasts.add_toast(adw::Toast::new(text));
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
                // No "Saved" toast: the subtitle clearing is the confirmation, and routine
                // success asking to be noticed is noise. A save that *fails* still says so.
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
            .filters(&text_save_filters())
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
                    None => ui.closing.set(false),
                }
            ),
        );
    }

    fn open(self: &Rc<Self>) {
        let dialog = gtk::FileDialog::builder()
            .title("Open")
            .filters(&text_filters())
            .build();
        dialog.open(
            Some(&self.window),
            gio::Cancellable::NONE,
            glib::clone!(
                #[strong(rename_to = ui)]
                self,
                move |result| {
                    if let Some(path) = result.ok().and_then(|file| file.path()) {
                        ui.load(&path);
                    }
                }
            ),
        );
    }

    fn load(self: &Rc<Self>, path: &Path) {
        self.loading.set(true);
        match self.app.open_file(path) {
            Ok(()) => {
                *self.path.borrow_mut() = Some(path.to_owned());
                self.banner.set_revealed(false);
                self.doc.reset();
                remember_recent(path);
            }
            Err(error) => {
                self.loading.set(false);
                // Cross-app handoff (`doc/suite.md`): opening a spreadsheet here must not
                // fail obscurely. `grind_core::kind` knows before any parsing does.
                match is_spreadsheet(path) {
                    true => self.offer_handoff(path),
                    false => self.toast(&format!("Could not open: {error}")),
                }
            }
        }
    }

    /// The banner that says "this is a spreadsheet", with the button that opens it there.
    fn offer_handoff(self: &Rc<Self>, path: &Path) {
        *self.handoff.borrow_mut() = Some(path.to_owned());
        self.banner
            .set_title(&format!("{} is a spreadsheet.", document_name(Some(path))));
        self.banner.set_button_label(Some("Open in Sheet"));
        self.banner.set_revealed(true);
    }

    fn hand_over(self: &Rc<Self>) {
        let Some(path) = self.handoff.borrow_mut().take() else {
            return;
        };
        self.banner.set_revealed(false);
        match std::process::Command::new(SHEET_APP).arg(&path).spawn() {
            Ok(_) => {}
            // Saying plainly that the sibling is not installed, which is what `doc/suite.md`
            // asks for — the alternative is a button that silently does nothing.
            Err(_) => self.toast(&format!("{SHEET_APP} is not installed")),
        }
    }

    fn new_document(self: &Rc<Self>) {
        // There is no `App::reset`, and there does not need to be: an empty document written
        // and read back is one, through the same path a file takes.
        let Ok(bytes) = grind_text::write_bytes(&grind_text::Document::default(), Form::Flat)
        else {
            return;
        };
        self.loading.set(true);
        if self.app.open_bytes("untitled.fodt", &bytes).is_ok() {
            *self.path.borrow_mut() = None;
            let _ = self.app.insert(0, BlockKind::Paragraph, "");
            self.doc.reset();
        }
    }

    // --- the document, as a whole ---

    /// The outline as a list of addresses. `grind text outline`'s window, and the navigation
    /// primitive this document type has instead of a sheet tab bar.
    fn outline(self: &Rc<Self>) {
        let headings = self.app.outline();
        let list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .build();
        list.add_css_class("boxed-list");
        for heading in &headings {
            let row = adw::ActionRow::builder()
                .title(glib::markup_escape_text(&heading.text))
                .subtitle(heading.address())
                .activatable(true)
                .build();
            // Indented by level, because an outline that does not show depth is a list.
            row.set_margin_start(((heading.level.min(6) - 1) * 16) as i32);
            let address = heading.address();
            row.connect_activated(glib::clone!(
                #[strong(rename_to = ui)]
                self,
                move |_| {
                    if let Ok(caret) = view::caret_of(&ui.app, &address) {
                        ui.doc.go_to(caret);
                        ui.doc.grab_focus();
                    }
                }
            ));
            list.append(&row);
        }

        let body: gtk::Widget = match headings.is_empty() {
            true => {
                let empty = gtk::Label::builder()
                    .label("No headings yet. A heading is what makes a section addressable.")
                    .wrap(true)
                    .build();
                empty.add_css_class("dim-label");
                empty.upcast()
            }
            false => list.upcast(),
        };
        let page = gtk::ScrolledWindow::builder()
            .propagate_natural_height(true)
            .child(&body)
            .build();
        let dialog = adw::Dialog::builder()
            .title("Outline")
            .content_width(460)
            .content_height(520)
            .build();
        let view = adw::ToolbarView::builder().content(&page).build();
        view.add_top_bar(&adw::HeaderBar::new());
        page.set_margin_top(12);
        page.set_margin_bottom(12);
        page.set_margin_start(12);
        page.set_margin_end(12);
        dialog.set_child(Some(&view));
        dialog.present(Some(&self.window));
    }

    /// The other half of `grind text words`: what is in this document, counted.
    fn word_count(self: &Rc<Self>) {
        let counts = self.app.counts();
        self.toast(&format!(
            "{} words · {} characters · {} blocks · {} headings",
            counts.words, counts.characters, counts.blocks, counts.headings
        ));
    }

    /// Make the caret's block a heading of `level`, or a paragraph again at level 0 — the
    /// window's `grind text kind`.
    fn set_kind(self: &Rc<Self>, level: u32) {
        let kind = match level {
            0 => BlockKind::Paragraph,
            level => BlockKind::Heading { level },
        };
        if let Err(error) = self.app.set_kind(self.doc.caret().block, kind) {
            self.toast(&error.to_string());
        }
    }

    fn about(&self) {
        let about = adw::AboutDialog::builder()
            .application_name("Text")
            .application_icon(APP_ID)
            .developer_name("Florian Wilhelm")
            .version(env!("CARGO_PKG_VERSION"))
            .website("https://github.com/fwilhe2/grind")
            .license_type(gtk::License::Agpl30)
            .comments("An ODF-native word processor.")
            .debug_info(grind_core::build_info::describe(
                "grind-text-gtk",
                env!("CARGO_PKG_VERSION"),
            ))
            .build();
        about.present(Some(&self.window));
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
}

/// Every action the window has, with its accelerators. One table, read twice: once to wire
/// the actions and once by the menu.
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
        ("goto", &["<Control>g"][..], |ui| ui.goto.popup()),
        ("outline", &["<Control><Shift>o"][..], |ui| ui.outline()),
        ("words", &[][..], |ui| ui.word_count()),
        ("paragraph", &["<Control>0"][..], |ui| ui.set_kind(0)),
        ("heading-1", &["<Control>1"][..], |ui| ui.set_kind(1)),
        ("heading-2", &["<Control>2"][..], |ui| ui.set_kind(2)),
        ("heading-3", &["<Control>3"][..], |ui| ui.set_kind(3)),
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

    let structure = gio::Menu::new();
    structure.append(Some("Outline…"), Some("win.outline"));
    structure.append(Some("Go to Address"), Some("win.goto"));
    structure.append(Some("Word Count"), Some("win.words"));
    menu.append_section(None, &structure);

    let kinds = gio::Menu::new();
    kinds.append(Some("Paragraph"), Some("win.paragraph"));
    kinds.append(Some("Heading 1"), Some("win.heading-1"));
    kinds.append(Some("Heading 2"), Some("win.heading-2"));
    kinds.append(Some("Heading 3"), Some("win.heading-3"));
    menu.append_section(None, &kinds);

    let rest = gio::Menu::new();
    rest.append(Some("About Text"), Some("win.about"));
    menu.append_section(None, &rest);
    menu
}

/// The observer's end of the bridge: a token, because nothing from the page may cross a
/// `Send + Sync` trait.
struct Bridge(async_channel::Sender<()>);

impl Observer for Bridge {
    fn changed(&self) {
        let _ = self.0.send_blocking(());
    }
}

/// One filter, both extensions — an *open* dialog must not ask which physical form a document
/// the user is looking for happens to be in, because they do not know and it does not matter.
fn text_filters() -> gio::ListStore {
    let filter = gtk::FileFilter::new();
    filter.set_name(Some("OpenDocument Text"));
    filter.add_pattern("*.fodt");
    filter.add_pattern("*.odt");

    let filters = gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&filter);
    filters
}

/// Saving is the other case: the question *does* have an answer there, and `doc/flat-first.md`
/// is that answer. Two filters rather than one, flat first, so the default selection writes the
/// form that diffs and choosing the package is one deliberate click away.
fn text_save_filters() -> gio::ListStore {
    let filters = gio::ListStore::new::<gtk::FileFilter>();
    for (name, pattern) in [
        ("OpenDocument Text (flat XML)", "*.fodt"),
        ("OpenDocument Text (package)", "*.odt"),
    ] {
        let filter = gtk::FileFilter::new();
        filter.set_name(Some(name));
        filter.add_pattern(pattern);
        filters.append(&filter);
    }
    filters
}

fn remember_recent(path: &Path) {
    gtk::RecentManager::default().add_item(&gio::File::for_path(path).uri());
}

fn document_name(path: Option<&Path>) -> String {
    path.and_then(|p| p.file_name()).map_or_else(
        || "Untitled".to_owned(),
        |name| name.to_string_lossy().into_owned(),
    )
}

/// Whether a file is a spreadsheet, decided from its bytes rather than its name — which is
/// what `grind_core::kind` is for, and it answers before any parsing.
fn is_spreadsheet(path: &Path) -> bool {
    std::fs::read(path)
        .ok()
        .and_then(|bytes| kind(&bytes))
        .is_some_and(|found| found == DocumentKind::Spreadsheet)
}

fn describe_kind(kind: &BlockKind, style: Option<&str>) -> String {
    let name = match kind {
        BlockKind::Paragraph => "paragraph".to_owned(),
        BlockKind::Heading { level } => format!("heading {level}"),
        BlockKind::ListItem { depth } => format!("list item, depth {depth}"),
    };
    match style {
        Some(style) => format!("{name} ({style})"),
        None => name,
    }
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

/// Draw one frame, write it and quit — how a machine checks that the view still draws.
///
/// Not a user feature. A refactor is proved one when the PNG comes back byte-identical.
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
            Err(error) => eprintln!("grind-text-gtk: --render-to: {error}"),
        }
        window.close();
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The status bar's vocabulary — the one thing in this file that is not a widget call.
    #[test]
    fn a_block_is_described_by_its_kind_and_its_style() {
        assert_eq!(describe_kind(&BlockKind::Paragraph, None), "paragraph");
        assert_eq!(
            describe_kind(&BlockKind::Heading { level: 2 }, None),
            "heading 2"
        );
        assert_eq!(
            describe_kind(&BlockKind::ListItem { depth: 1 }, Some("Quote")),
            "list item, depth 1 (Quote)"
        );
    }

    #[test]
    fn a_document_is_named_by_its_file_and_untitled_without_one() {
        assert_eq!(
            document_name(Some(Path::new("/tmp/report.fodt"))),
            "report.fodt"
        );
        assert_eq!(document_name(None), "Untitled");
    }

    /// The cross-app handoff decides on the *bytes*, never the extension — a `.odt` that is
    /// really a spreadsheet is still a spreadsheet (`core/src/kind.rs`).
    #[test]
    fn a_spreadsheet_is_recognised_by_its_content() {
        let dir = std::env::temp_dir().join("grind-text-gtk-tests");
        std::fs::create_dir_all(&dir).expect("a temp directory");
        let path = dir.join("liar.fodt");
        let bytes = grind_sheet_bytes();
        std::fs::write(&path, bytes).expect("writes");
        assert!(is_spreadsheet(&path));

        let text = grind_text::write_bytes(&grind_text::Document::default(), Form::Flat)
            .expect("writes a text document");
        std::fs::write(&path, text).expect("writes");
        assert!(!is_spreadsheet(&path));
        let _ = std::fs::remove_file(&path);
    }

    /// A flat spreadsheet, spelled out rather than depended on: this crate must not have
    /// `grind-sheet` in its manifest, and the whole point of `kind` is that recognising one
    /// needs nothing but the bytes.
    fn grind_sheet_bytes() -> &'static str {
        r#"<?xml version="1.0" encoding="UTF-8"?>
<office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0"
    office:version="1.4"
    office:mimetype="application/vnd.oasis.opendocument.spreadsheet">
  <office:body><office:spreadsheet/></office:body>
</office:document>"#
    }

    /// `doc/flat-first.md`, at the one keystroke that matters: the name a Save As dialog offers
    /// for a document that has never been saved. A document that already has a path keeps it,
    /// so nothing is ever converted behind somebody's back.
    #[test]
    fn an_unnamed_document_is_offered_the_flat_extension() {
        assert_eq!(save_name(None), "Untitled.fodt");
        assert_eq!(save_name(Some(Path::new("/tmp/report.odt"))), "report.odt");
        assert_eq!(
            save_name(Some(Path::new("/tmp/report.fodt"))),
            "report.fodt"
        );
    }
}
