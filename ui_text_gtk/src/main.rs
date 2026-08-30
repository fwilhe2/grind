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

mod code;
mod geom;
mod keymap;
mod lint;
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
use grind_text::{App, BlockKind, CharStyle, Form};
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
    // `--overlay names` turns `doc/view-modes.md` §3.6's bookmark anchors on for that frame,
    // so the mode is assertable the same way the rest of this widget is. Not a user feature
    // either — the window's own menu item is.
    let mut names = false;
    let mut args = std::env::args_os().skip(1);
    while let Some(arg) = args.next() {
        match arg {
            arg if arg == "--render-to" => {
                render_to = Some(PathBuf::from(
                    args.next().unwrap_or_else(|| "document.png".into()),
                ));
            }
            arg if arg == "--overlay" => match args.next().as_deref().and_then(|a| a.to_str()) {
                Some("names") => names = true,
                other => {
                    eprintln!(
                        "grind-text-gtk: --overlay takes names, not {}",
                        other.unwrap_or("nothing")
                    );
                    return ExitCode::FAILURE;
                }
            },
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
        if names {
            ui.doc.set_names(true);
        }
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
    /// The four character-formatting toggles. Each reads as pressed when the selection
    /// agrees it is on, and both reading and writing go through the same `App` the rest of
    /// this file does (`App::char_style`/`set_char_style`) — no toolbar has its own idea of
    /// what bold means.
    bold: gtk::ToggleButton,
    italic: gtk::ToggleButton,
    underline: gtk::ToggleButton,
    strike: gtk::ToggleButton,
    /// Guards [`Ui::refresh`]'s own `set_active` calls from being read back as a click —
    /// without it, painting the toolbar's state would immediately rewrite the document it
    /// was reporting on.
    updating: Cell<bool>,
    /// The two pages of the window: the document, and its projection (`doc/dsl.md` §6, D9).
    stack: gtk::Stack,
    source: gtk::TextView,
    /// The code view line the caret's block is on, as last marked.
    ///
    /// `updating` is not enough on its own: GTK does not always deliver
    /// `notify::cursor-position` inside the call that caused it, so a handler that answers by
    /// touching the cursor is a loop the latch never sees the re-entry of. This makes the
    /// handler *idempotent* instead — the same line twice does nothing — which holds however the
    /// signal is scheduled.
    marked: Cell<Option<usize>>,
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

        // The code view, on the other page of a stack (`doc/dsl.md` §6.1: "a `GtkTextView` in a
        // `GtkStack`, with the stack switcher as the Delphi tab"). A stack rather than a paned
        // split: §6.2 is right that a split is what a person eventually wants, and it is also a
        // second viewport to keep in step. What pays for itself first is the correspondence, and
        // one page at a time carries it.
        let source = code::build();
        let source_scroller = gtk::ScrolledWindow::builder()
            .hexpand(true)
            .vexpand(true)
            .child(&source)
            .build();
        let stack = gtk::Stack::new();
        stack.add_named(&scroller, Some("document"));
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

        // The formatting toolbar — a second top bar rather than crowded into the header, the
        // way a spreadsheet's format strip sits under its own header (`ui_sheet_gtk`). Every
        // button here writes through `App::set_char_style`, so a run it touches survives a
        // LibreOffice round-trip the same way typing does (R6).
        let bold = gtk::ToggleButton::builder()
            .icon_name("format-text-bold-symbolic")
            .tooltip_text("Bold")
            .build();
        let italic = gtk::ToggleButton::builder()
            .icon_name("format-text-italic-symbolic")
            .tooltip_text("Italic")
            .build();
        let underline = gtk::ToggleButton::builder()
            .icon_name("format-text-underline-symbolic")
            .tooltip_text("Underline")
            .build();
        let strike = gtk::ToggleButton::builder()
            .icon_name("format-text-strikethrough-symbolic")
            .tooltip_text("Strikethrough")
            .build();
        let format_group = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        format_group.add_css_class("linked");
        for button in [&bold, &italic, &underline, &strike] {
            format_group.append(button);
        }
        let formatting = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(6)
            .margin_start(6)
            .margin_end(6)
            .margin_top(4)
            .margin_bottom(4)
            .build();
        formatting.append(&format_group);

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
        content.add_top_bar(&formatting);
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
            stack,
            source,
            marked: Cell::new(None),
            title,
            toasts,
            banner,
            status,
            address,
            goto,
            undo,
            redo,
            bold,
            italic,
            underline,
            strike,
            updating: Cell::new(false),
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

        for (button, mutate) in [
            (&self.bold, CharStyle::set_bold as fn(&mut CharStyle, bool)),
            (&self.italic, CharStyle::set_italic),
            (&self.underline, CharStyle::set_underlined),
            (&self.strike, CharStyle::set_struck),
        ] {
            button.connect_toggled(glib::clone!(
                #[strong(rename_to = ui)]
                self,
                move |button| {
                    // `refresh` sets these to reflect the document; only a click — the user
                    // actually toggling one — should write anything back.
                    if !ui.updating.get() {
                        ui.apply_char_style(mutate, button.is_active());
                    }
                }
            ));
        }

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

        // `doc/view-modes.md` §3.6, stateful because it is a *mode* rather than a verb: the
        // menu item carries its own checkmark, and the same key turns it off. Nothing is
        // written either way, which is why it needs no confirmation and leaves no undo entry.
        let names =
            gio::SimpleAction::new_stateful("show-names", None, &self.doc.names().to_variant());
        names.connect_activate(glib::clone!(
            #[strong(rename_to = ui)]
            self,
            move |action, _| {
                let on = !ui.doc.names();
                ui.doc.set_names(on);
                action.set_state(&on.to_variant());
            }
        ));
        self.window.add_action(&names);
        application.set_accels_for_action("win.show-names", &["<Control><Shift>n"]);

        // `doc/dsl.md` §6, D9 — the document as its projection, on the other page of the stack.
        // Stateful like `show-names` and for the same reason: it is a *reading* of the document,
        // it writes nothing, and the same item turns it off.
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

        // Moving the cursor in the source selects the block that line projects — §6.2's map in
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

    /// Switch between the document and its source.
    fn show_source(self: &Rc<Self>, on: bool) {
        self.stack.set_visible_child_name(match on {
            true => "source",
            false => "document",
        });
        match on {
            true => {
                self.fill_source();
                let _ = self.source.grab_focus();
            }
            false => {
                gtk::prelude::GtkWindowExt::set_focus(&self.window, Some(&self.doc));
            }
        }
    }

    /// Paint the projection, with the caret's own block marked.
    ///
    /// `updating` is raised for the same reason the toolbar raises it: placing the cursor in the
    /// buffer fires `cursor-position`, and answering that by moving the document's caret would
    /// be the view rewriting the thing it is reporting on.
    fn fill_source(self: &Rc<Self>) {
        let projection = self.app.project();
        let line = projection.line_of(&grind_text::loc::format(self.doc.caret().block));
        self.updating.set(true);
        code::fill(&self.source, &projection, line);
        self.marked.set(line);
        self.updating.set(false);
    }

    /// The source's cursor moved: put the document's caret in the block that line projects.
    ///
    /// The span map may hand back `p12`, `#intro` or `§2.1.3`, and `loc::parse` takes all three,
    /// so this needs no vocabulary of its own — `loc.rs` earning its keep again.
    fn source_moved(self: &Rc<Self>) {
        if self.updating.get() || self.stack.visible_child_name().as_deref() != Some("source") {
            return;
        }
        let line = code::line_at_cursor(&self.source);
        // Already answered — see `marked`. Without this the window hangs.
        if self.marked.get() == Some(line) {
            return;
        }
        self.marked.set(Some(line));
        let projection = self.app.project();
        let Some(address) = projection.address_on_line(line) else {
            return;
        };
        let Ok(caret) = view::caret_of(&self.app, address) else {
            return;
        };
        self.doc.go_to(caret);
        // The tag only — moving the cursor from the handler that runs because the cursor moved
        // is the loop this guard exists for. See `code::mark`.
        code::mark(&self.source, line);
        self.refresh();
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
        self.refresh_formatting();

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

    /// `grind text lint` with a list in front of it (`doc/dsl.md` §4.3, D6). The jump is this
    /// window's own — an address goes through `view::caret_of`, which is `loc::parse` plus the
    /// core's resolution, exactly as the outline dialog and the go-to box already do.
    fn lint(self: &Rc<Self>) {
        lint::present(
            &self.window,
            &self.app,
            glib::clone!(
                #[strong(rename_to = ui)]
                self,
                move |address: &str| {
                    if let Ok(caret) = view::caret_of(&ui.app, address) {
                        ui.doc.go_to(caret);
                        ui.doc.grab_focus();
                    }
                }
            ),
        );
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

    /// The toolbar's other half of `refresh`: what the four toggles show, read from the
    /// selection rather than kept as state of their own — there is exactly one fact anywhere
    /// about whether a run is bold, and it is in the document (`App::char_style`).
    fn refresh_formatting(self: &Rc<Self>) {
        let selected = self.doc.selection();
        let style = selected
            .and_then(|(from, to)| self.app.char_style(from, to).ok())
            .unwrap_or_default();
        self.updating.set(true);
        for button in [&self.bold, &self.italic, &self.underline, &self.strike] {
            button.set_sensitive(selected.is_some());
        }
        self.bold.set_active(style.is_bold());
        self.italic.set_active(style.is_italic());
        self.underline.set_active(style.is_underlined());
        self.strike.set_active(style.is_struck());
        self.updating.set(false);
    }

    /// One toolbar toggle, applied to the current selection. `mutate` sets the one property
    /// that button owns — `CharStyle::set_bold` and its three siblings — on top of what the
    /// selection already agrees about, so toggling Italic on a bold-and-italic run leaves the
    /// bold alone.
    fn apply_char_style(self: &Rc<Self>, mutate: fn(&mut CharStyle, bool), on: bool) {
        let Some((from, to)) = self.doc.selection() else {
            // The toggle is insensitive with no selection, so a click here would have to be
            // a stray key event rather than a person — nothing to toast about.
            return;
        };
        let mut style = self.app.char_style(from, to).unwrap_or_default();
        mutate(&mut style, on);
        if let Err(error) = self.app.set_char_style(from, to, &style) {
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
        // F8, the "next problem" key, and the same one `grind-sheet-gtk` uses — one suite, one
        // key for one job (`doc/dsl.md` §4.3, D6).
        ("lint", &["F8"][..], |ui| ui.lint()),
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
    structure.append(Some("Show Bookmarks"), Some("win.show-names"));
    structure.append(Some("Show Source"), Some("win.show-source"));
    structure.append(Some("Word Count"), Some("win.words"));
    structure.append(Some("Check Document"), Some("win.lint"));
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

/// One filter, all three extensions — an *open* dialog must not ask which physical form a
/// document the user is looking for happens to be in, because they do not know and it does not
/// matter.
fn text_filters() -> gio::ListStore {
    let filter = gtk::FileFilter::new();
    filter.set_name(Some("OpenDocument Text"));
    filter.add_pattern("*.fodt");
    filter.add_pattern("*.odt");
    // The third physical form (`doc/dsl.md` D2/D4). The window needed no other change to open
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
fn text_save_filters() -> gio::ListStore {
    let filters = gio::ListStore::new::<gtk::FileFilter>();
    for (name, pattern) in [
        ("OpenDocument Text (flat XML)", "*.fodt"),
        ("OpenDocument Text (package)", "*.odt"),
        ("Grind projection", "*.grind"),
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
