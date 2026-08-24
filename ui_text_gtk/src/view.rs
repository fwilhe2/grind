// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The document view: a custom widget that draws blocks and a caret, and owns neither.
//!
//! `ui_gtk/src/grid.rs`'s counterpart, one document type over. Every paint asks
//! [`App::get_viewport`] for the blocks that fall on screen and [`App::layout_block`] for
//! their lines, draws them, and throws both away (doc/plan.md rule 1). The only state here is
//! presentation: where the caret is, and what column it is trying to keep while moving by
//! lines.
//!
//! **Not `GtkTextView`.** That widget owns a `GtkTextBuffer`, which is a second copy of the
//! document with its own undo stack, its own idea of what a paragraph is and no notion of a
//! `text:h` — rule 1's trap in its most tempting form. A widget drawing in `snapshot()` from
//! the core's own layout is more code once and keeps one document.
//!
//! **Where the editing model is, and is not.** Every motion is answered by the core:
//! Down-arrow is [`App::caret_line`], Home and End are [`App::caret_line_bounds`], a click is
//! [`grind_core::layout::Layout::offset_at`], typing is [`App::insert_text`], Enter is
//! [`App::split_block`], Backspace at the front of a block is what [`App::erase`] across a
//! boundary already does. This file decides *which* question to ask and where to draw the
//! answer — exactly the division `ui_tui/src/text/app.rs` makes, in a different unit, which
//! is what `doc/text-layout.md` chose Path C for.
//!
//! Typing goes through `GtkIMMulticontext`, so dead keys, compose sequences and input methods
//! work. [`crate::keymap`] never sees a printable character.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use libadwaita::gtk;
use libadwaita::prelude::*;
use libadwaita::subclass::prelude::ObjectSubclassIsExt;

use grind_text::{App, Caret};
use gtk::glib;

use crate::geom::Flow;

glib::wrapper! {
    pub struct Doc(ObjectSubclass<imp::Doc>)
        @extends gtk::Widget,
        @implements gtk::Scrollable, gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Doc {
    pub fn new(app: Arc<App>) -> Self {
        let doc: Self = glib::Object::builder().build();
        doc.imp().app.replace(Some(app));
        doc
    }

    /// The document changed: forget everything measured from it and repaint.
    ///
    /// Called from the observer, so it covers this widget's own edits too — they reach the
    /// core the same way anything else does (doc/plan.md rule 3).
    pub fn invalidate(&self) {
        self.imp().flow.replace(None);
        self.imp().clamp_caret();
        // The document's height changed, so the scrollbar has to be sized again — which is
        // what allocation does, and it has the width and height to do it with.
        self.queue_allocate();
        self.queue_draw();
    }

    /// A different document: back to the top, with no goal column carried over.
    pub fn reset(&self) {
        let imp = self.imp();
        imp.caret.set(Caret {
            block: 0,
            offset: 0,
        });
        imp.goal_x.set(None);
        if let Some(adjustment) = imp.vadjustment.borrow().as_ref() {
            adjustment.set_value(0.0);
        }
        self.invalidate();
    }

    pub fn caret(&self) -> Caret {
        self.imp().caret.get()
    }

    /// Told whenever the caret moves — the status bar's readout.
    pub fn connect_moved(&self, f: impl Fn(Caret) + 'static) {
        self.imp().on_moved.borrow_mut().push(Box::new(f));
    }

    /// Told when an edit was refused, with the core's own message. A toast, never a dialog:
    /// nothing here is a question.
    pub fn connect_notice(&self, f: impl Fn(String) + 'static) {
        self.imp().on_notice.borrow_mut().push(Box::new(f));
    }

    /// Set the caret from an address the user typed — `p12`, `#intro`, `§2.1.3`.
    pub fn go_to(&self, caret: Caret) {
        self.imp().move_caret(caret, true);
    }
}

mod imp {
    use super::*;

    use grind_core::layout::Layout;
    use grind_text::{BlockKind, loc};
    use gtk::graphene;
    use gtk::pango;
    use gtk::subclass::prelude::*;

    use crate::geom;
    use crate::keymap::{self, Action, Key, Mods, Motion};
    use crate::metrics::Faces;
    use crate::theme::Palette;

    type NoticeHook = Box<dyn Fn(String)>;
    type MovedHook = Box<dyn Fn(Caret)>;

    /// How thick the caret is, and how far a list bullet sits left of its text.
    const CARET: f64 = 1.5;
    const BULLET_GAP: f64 = 14.0;

    pub struct Doc {
        pub app: RefCell<Option<Arc<App>>>,
        pub caret: Cell<Caret>,
        /// The column the caret is trying to keep while moving by lines — see
        /// [`App::caret_line`]. Cleared by any horizontal move, which is what makes walking
        /// down through a short line and out the other side come back where it started.
        pub goal_x: Cell<Option<f32>>,
        /// Every block's box, cached against the width it was measured at.
        ///
        /// ponytail: rebuilt from scratch whenever the document or the width changes, which
        /// lays out every block in the document rather than the ones that changed. Bounded by
        /// the document rather than by the screen, unlike everything else in this file; the
        /// upgrade path is a per-`BlockId` cache, and the reason not to have one yet is that
        /// it needs an invalidation rule the core does not hand out.
        pub flow: RefCell<Option<(f64, Rc<Flow>)>>,
        pub faces: RefCell<Option<Rc<Faces>>>,
        pub palette: Cell<Option<Palette>>,
        pub hadjustment: RefCell<Option<gtk::Adjustment>>,
        pub vadjustment: RefCell<Option<gtk::Adjustment>>,
        pub hscroll_policy: Cell<gtk::ScrollablePolicy>,
        pub vscroll_policy: Cell<gtk::ScrollablePolicy>,
        pub im: gtk::IMMulticontext,
        pub on_notice: RefCell<Vec<NoticeHook>>,
        pub on_moved: RefCell<Vec<MovedHook>>,
    }

    // Spelled out rather than derived: neither `Caret` nor `ScrollablePolicy` has a
    // `Default`, and the caret's is a decision anyway — a new view starts at the top of the
    // document, which is the one position every document has.
    impl Default for Doc {
        fn default() -> Self {
            Doc {
                app: RefCell::new(None),
                caret: Cell::new(Caret {
                    block: 0,
                    offset: 0,
                }),
                goal_x: Cell::new(None),
                flow: RefCell::new(None),
                faces: RefCell::new(None),
                palette: Cell::new(None),
                hadjustment: RefCell::new(None),
                vadjustment: RefCell::new(None),
                hscroll_policy: Cell::new(gtk::ScrollablePolicy::Minimum),
                vscroll_policy: Cell::new(gtk::ScrollablePolicy::Minimum),
                im: gtk::IMMulticontext::new(),
                on_notice: RefCell::new(Vec::new()),
                on_moved: RefCell::new(Vec::new()),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Doc {
        const NAME: &'static str = "GrindTextDoc";
        type Type = super::Doc;
        type ParentType = gtk::Widget;
        type Interfaces = (gtk::Scrollable,);

        fn class_init(klass: &mut Self::Class) {
            klass.set_css_name("grinddoc");
            // `TextBox` is the role for an editable region of text, which is what this is
            // even though it draws itself.
            klass.set_accessible_role(gtk::AccessibleRole::TextBox);
        }
    }

    impl ObjectImpl for Doc {
        // The four properties `GtkScrollable` requires, overridden by hand for the reason
        // `ui_gtk/src/grid.rs` gives: the `Properties` derive's spelling has churned between
        // gtk4-rs releases and this shape does not move.
        fn properties() -> &'static [glib::ParamSpec] {
            static PROPERTIES: std::sync::OnceLock<Vec<glib::ParamSpec>> =
                std::sync::OnceLock::new();
            PROPERTIES.get_or_init(|| {
                vec![
                    glib::ParamSpecOverride::for_interface::<gtk::Scrollable>("hadjustment"),
                    glib::ParamSpecOverride::for_interface::<gtk::Scrollable>("vadjustment"),
                    glib::ParamSpecOverride::for_interface::<gtk::Scrollable>("hscroll-policy"),
                    glib::ParamSpecOverride::for_interface::<gtk::Scrollable>("vscroll-policy"),
                ]
            })
        }

        fn set_property(&self, _id: usize, value: &glib::Value, pspec: &glib::ParamSpec) {
            match pspec.name() {
                "hadjustment" => {
                    self.hadjustment.replace(value.get().ok());
                }
                "vadjustment" => {
                    let adjustment: Option<gtk::Adjustment> = value.get().ok();
                    // A scroll changes nothing in the document, so nothing else would ask
                    // for the repaint that draws the page it moved to.
                    if let Some(adjustment) = &adjustment {
                        adjustment.connect_value_changed(glib::clone!(
                            #[weak(rename_to = doc)]
                            self.obj(),
                            move |_| doc.queue_draw()
                        ));
                    }
                    self.vadjustment.replace(adjustment);
                }
                "hscroll-policy" => self.hscroll_policy.set(value.get().unwrap()),
                "vscroll-policy" => self.vscroll_policy.set(value.get().unwrap()),
                other => unimplemented!("property {other}"),
            }
            self.obj().queue_allocate();
        }

        fn property(&self, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            match pspec.name() {
                "hadjustment" => self.hadjustment.borrow().to_value(),
                "vadjustment" => self.vadjustment.borrow().to_value(),
                "hscroll-policy" => self.hscroll_policy.get().to_value(),
                "vscroll-policy" => self.vscroll_policy.get().to_value(),
                other => unimplemented!("property {other}"),
            }
        }

        fn constructed(&self) {
            self.parent_constructed();
            let widget = self.obj();
            widget.set_focusable(true);

            self.im.set_client_widget(Some(&*widget));
            self.im.connect_commit(glib::clone!(
                #[weak(rename_to = doc)]
                widget,
                move |_, text| doc.imp().type_text(text)
            ));

            let keys = gtk::EventControllerKey::new();
            // Everything this shell does not claim travels on to the input method, which is
            // what turns a keystroke into the text that was actually meant.
            keys.set_im_context(Some(&self.im));
            keys.connect_key_pressed(glib::clone!(
                #[weak(rename_to = doc)]
                widget,
                #[upgrade_or]
                glib::Propagation::Proceed,
                move |_, keyval, _, state| doc.imp().key_pressed(keyval, state)
            ));
            widget.add_controller(keys);

            // The input method has to be told about focus, or a compose sequence started in
            // another window finishes in this one.
            let focus = gtk::EventControllerFocus::new();
            focus.connect_enter(glib::clone!(
                #[weak(rename_to = doc)]
                widget,
                move |_| doc.imp().im.focus_in()
            ));
            focus.connect_leave(glib::clone!(
                #[weak(rename_to = doc)]
                widget,
                move |_| doc.imp().im.focus_out()
            ));
            widget.add_controller(focus);

            let click = gtk::GestureClick::new();
            click.connect_pressed(glib::clone!(
                #[weak(rename_to = doc)]
                widget,
                move |_, _, x, y| {
                    doc.grab_focus();
                    doc.imp().click(x, y);
                }
            ));
            widget.add_controller(click);
        }
    }

    impl WidgetImpl for Doc {
        /// A scrollable asks for nothing and takes what it is given.
        fn measure(&self, _orientation: gtk::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
            (0, 0, -1, -1)
        }

        fn size_allocate(&self, width: i32, height: i32, _baseline: i32) {
            // A resize re-wraps every paragraph, so the height the scrollbar is sized against
            // is only knowable here.
            let flow = self.flow(f64::from(width));
            configure(
                self.vadjustment.borrow().as_ref(),
                f64::from(height).max(1.0),
                flow.height() + geom::MARGIN,
                self.faces().body().height(),
            );
        }

        fn realize(&self) {
            self.parent_realize();
            self.restyle();
        }

        /// A theme switch or a font change: everything derived from the style goes, including
        /// every line break measured with the old font.
        fn system_setting_changed(&self, setting: &gtk::SystemSetting) {
            self.parent_system_setting_changed(setting);
            self.restyle();
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let widget = self.obj();
            let width = f64::from(widget.width());
            let height = f64::from(widget.height());
            let Some(app) = self.app() else { return };
            let palette = self.palette();
            let faces = self.faces();
            let flow = self.flow(width);
            let (left, column) = geom::column(width);
            let scroll = self.scroll();

            snapshot.append_color(&palette.background, &rect(0.0, 0.0, width, height));

            let slots = flow.visible(scroll, scroll + height);
            let Some((first, last)) = slots.first().zip(slots.last()) else {
                return;
            };
            // One read for the whole paint, exactly as wide as the screen (rule 1).
            let viewport = app.get_viewport(first.index..last.index + 1);
            let caret = self.caret.get();

            for slot in slots {
                let Some(block) = viewport.get(slot.index) else {
                    continue;
                };
                let face = faces.of(&block.kind);
                let Ok(layout) = app.layout_block(slot.index, (column - slot.indent) as f32, face)
                else {
                    continue;
                };
                let text: Vec<char> = block.text.chars().collect();
                let x = left + slot.indent;
                let y = slot.top - scroll;

                if let BlockKind::ListItem { .. } = block.kind {
                    // A bullet is drawn rather than inserted: the character is not in the
                    // document, and putting one there would make it a character the caret
                    // could sit inside and a `p12+0` that means something else.
                    draw_at(
                        snapshot,
                        face.draw("\u{2022}"),
                        x - BULLET_GAP,
                        y,
                        palette.dim,
                    );
                }

                for line in layout.lines() {
                    let piece: String = text[line.start.min(text.len())..line.end.min(text.len())]
                        .iter()
                        .collect();
                    draw_at(
                        snapshot,
                        // A line's `end` includes the break that ended it, and a newline
                        // handed to Pango would start a second line inside this one.
                        face.draw(piece.trim_end_matches('\n')),
                        x,
                        y + f64::from(line.top),
                        palette.foreground,
                    );
                }

                if slot.index == caret.block && widget.is_focus() {
                    let line = layout.lines()[layout.line_at(caret.offset)];
                    snapshot.append_color(
                        &palette.accent,
                        &rect(
                            x + f64::from(layout.x_at(caret.offset)),
                            y + f64::from(line.top),
                            CARET,
                            f64::from(line.height),
                        ),
                    );
                }
            }
        }
    }

    impl ScrollableImpl for Doc {}

    impl Doc {
        pub fn app(&self) -> Option<Arc<App>> {
            self.app.borrow().clone()
        }

        fn scroll(&self) -> f64 {
            self.vadjustment
                .borrow()
                .as_ref()
                .map_or(0.0, |a| a.value())
        }

        fn palette(&self) -> Palette {
            if let Some(palette) = self.palette.get() {
                return palette;
            }
            let palette = Palette::of(&*self.obj());
            self.palette.set(Some(palette));
            palette
        }

        pub fn faces(&self) -> Rc<Faces> {
            if let Some(faces) = self.faces.borrow().as_ref() {
                return faces.clone();
            }
            let faces = Rc::new(Faces::new(&self.obj().pango_context()));
            self.faces.replace(Some(faces.clone()));
            faces
        }

        /// Drop everything derived from the style and derive it again.
        fn restyle(&self) {
            self.palette.set(None);
            self.faces.replace(None);
            // Line breaking is font-dependent, so a new font is a new set of lines.
            self.flow.replace(None);
            self.obj().queue_allocate();
            self.obj().queue_draw();
        }

        /// Every block's box, measured at this width — cached, because a scroll must not
        /// re-lay-out the document.
        pub fn flow(&self, width: f64) -> Rc<Flow> {
            let (_, column) = geom::column(width);
            if let Some((measured, flow)) = self.flow.borrow().as_ref()
                && *measured == column
            {
                return flow.clone();
            }
            let flow = Rc::new(self.build_flow(column));
            self.flow.replace(Some((column, flow.clone())));
            flow
        }

        fn build_flow(&self, column: f64) -> Flow {
            // The page's top margin is part of the flow rather than a fixed band, so it
            // scrolls away with the text the way the top of a page does.
            let mut flow = Flow::new(geom::MARGIN);
            let Some(app) = self.app() else { return flow };
            let faces = self.faces();
            let viewport = app.get_viewport(0..app.block_count());
            for block in viewport.iter() {
                let face = faces.of(&block.kind);
                let indent = indent_of(&block.kind);
                let height = app
                    .layout_block(block.index, (column - indent) as f32, face)
                    .map(|layout| f64::from(layout.height()))
                    .unwrap_or_else(|_| face.height());
                let space = match block.kind {
                    BlockKind::Heading { .. } => geom::HEADING_GAP,
                    _ => geom::GAP,
                };
                flow.push(block.index, height, indent, space, geom::GAP);
            }
            flow
        }

        /// One block's lines, measured in its own face — what a caret operation is asked in.
        ///
        /// Every caller needs the same three things and getting one of them from a different
        /// block's face would put the caret in the wrong place, so they are fetched together.
        pub fn measured(&self, index: usize) -> Option<(Layout, Rc<Faces>, BlockKind)> {
            let app = self.app()?;
            let kind = app.get_viewport(index..index + 1).get(index)?.kind.clone();
            let faces = self.faces();
            let (_, column) = geom::column(f64::from(self.obj().width()));
            let width = (column - indent_of(&kind)) as f32;
            let layout = app.layout_block(index, width, faces.of(&kind)).ok()?;
            Some((layout, faces, kind))
        }

        /// The width and metrics a *line* operation is asked in, for the caret's own block.
        ///
        /// ponytail: [`App::caret_line`] takes one width and one provider for a motion that
        /// may cross into a block set in a different face, so Down-arrow out of a heading
        /// lands using the heading's metrics. Invisible for a caret in the middle of a line
        /// and wrong by a few characters at the ends. The fix is a core change — a provider
        /// looked up per block rather than passed once — and it is written down in
        /// `doc/text-shell.md` rather than worked around here, because working around it
        /// would mean this shell doing its own line arithmetic.
        fn line_context(&self) -> Option<(f32, Rc<Faces>, BlockKind)> {
            let (_, faces, kind) = self.measured(self.caret.get().block)?;
            let (_, column) = geom::column(f64::from(self.obj().width()));
            Some(((column - indent_of(&kind)) as f32, faces, kind))
        }

        // --- input ---

        fn key_pressed(
            &self,
            keyval: gtk::gdk::Key,
            state: gtk::gdk::ModifierType,
        ) -> glib::Propagation {
            let mods = Mods {
                ctrl: state.contains(gtk::gdk::ModifierType::CONTROL_MASK),
                shift: state.contains(gtk::gdk::ModifierType::SHIFT_MASK),
            };
            // A modifier chord belongs to the window's actions (Ctrl+S, Ctrl+Z) unless the
            // map claims it, and Ctrl+Home is the one that is claimed.
            let Some(action) = keymap::action_for(key_of(keyval), mods) else {
                return glib::Propagation::Proceed;
            };
            match action {
                Action::Move(motion) => self.go(motion),
                Action::Split => self.split(),
                Action::EraseBack => self.erase_back(),
                Action::EraseForward => self.erase_forward(),
            }
            glib::Propagation::Stop
        }

        /// Every motion, routed to the core.
        pub fn go(&self, motion: Motion) {
            let Some(app) = self.app() else { return };
            if app.block_count() == 0 {
                return;
            }
            let caret = self.caret.get();
            let Some((width, faces, kind)) = self.line_context() else {
                return;
            };
            let metrics = faces.of(&kind);
            match motion {
                Motion::Char(delta) => {
                    self.goal_x.set(None);
                    self.move_caret(self.stepped(&app, delta), true);
                }
                Motion::Line(steps) | Motion::Page(steps) => {
                    // A page is however many body lines fit, less one so that the line you
                    // were reading is still on screen after the jump.
                    let lines = match motion {
                        Motion::Page(_) => {
                            let fit = f64::from(self.obj().height()) / faces.body().height();
                            (fit as isize - 1).max(1)
                        }
                        _ => 1,
                    };
                    let delta = steps as isize * lines;
                    // Remembered across a run of Down presses, which is what `goal_x` is for.
                    let goal = match self.goal_x.get() {
                        Some(x) => x,
                        None => app.caret_x(caret, width, metrics).unwrap_or(0.0),
                    };
                    self.goal_x.set(Some(goal));
                    if let Ok(moved) = app.caret_line(caret, delta, goal, width, metrics) {
                        self.move_caret(moved, false);
                    }
                }
                Motion::LineStart | Motion::LineEnd => {
                    self.goal_x.set(None);
                    if let Ok((start, end)) = app.caret_line_bounds(caret, width, metrics) {
                        self.move_caret(
                            match motion {
                                Motion::LineStart => start,
                                _ => end,
                            },
                            true,
                        );
                    }
                }
                Motion::DocStart => self.move_caret(
                    Caret {
                        block: 0,
                        offset: 0,
                    },
                    true,
                ),
                Motion::DocEnd => {
                    let block = app.block_count() - 1;
                    self.move_caret(
                        Caret {
                            block,
                            offset: self.block_len(&app, block),
                        },
                        true,
                    );
                }
            }
        }

        /// One character left or right, rolling onto the neighbouring block at either end.
        ///
        /// The only arithmetic in this file, and it is over *characters* rather than over
        /// layout — walking off the end of a block is a document fact, not a line one.
        fn stepped(&self, app: &App, delta: i32) -> Caret {
            let mut caret = self.caret.get();
            if delta > 0 {
                if caret.offset < self.block_len(app, caret.block) {
                    caret.offset += 1;
                } else if caret.block + 1 < app.block_count() {
                    caret = Caret {
                        block: caret.block + 1,
                        offset: 0,
                    };
                }
            } else if caret.offset > 0 {
                caret.offset -= 1;
            } else if caret.block > 0 {
                caret = Caret {
                    block: caret.block - 1,
                    offset: self.block_len(app, caret.block - 1),
                };
            }
            caret
        }

        fn block_len(&self, app: &App, index: usize) -> usize {
            app.input_text(index)
                .map(|text| text.chars().count())
                .unwrap_or(0)
        }

        /// Where a click landed: the block under the pointer, and the offset nearest to x on
        /// the line it hit.
        fn click(&self, x: f64, y: f64) {
            let width = f64::from(self.obj().width());
            let flow = self.flow(width);
            let (left, _) = geom::column(width);
            let scroll = self.scroll();
            let Some(slot) = flow.at_y(y + scroll).and_then(|index| flow.slot(index)) else {
                return;
            };
            let Some((layout, _, _)) = self.measured(slot.index) else {
                return;
            };
            // Which line, then where on it: the same two questions in the same order the
            // core answers them, so a click and a Down-arrow land on the same offset.
            let line = line_at_y(&layout, y + scroll - slot.top);
            let offset = layout.offset_at(line, (x - left - slot.indent) as f32);
            self.move_caret(
                Caret {
                    block: slot.index,
                    offset,
                },
                true,
            );
        }

        // --- editing ---

        pub fn type_text(&self, text: &str) {
            let Some(app) = self.app() else { return };
            // A document with no blocks at all has nowhere to put a character, and the first
            // thing anybody does with an empty window is type into it.
            if app.block_count() == 0 {
                match app.insert(0, BlockKind::Paragraph, text) {
                    Ok(()) => self.move_caret(
                        Caret {
                            block: 0,
                            offset: text.chars().count(),
                        },
                        true,
                    ),
                    Err(error) => self.notice(error.to_string()),
                }
                return;
            }
            let caret = self.caret.get();
            match app.insert_text(caret, text) {
                Ok(()) => {
                    self.goal_x.set(None);
                    self.move_caret(
                        Caret {
                            block: caret.block,
                            offset: caret.offset + text.chars().count(),
                        },
                        true,
                    );
                }
                Err(error) => self.notice(error.to_string()),
            }
        }

        pub fn split(&self) {
            let Some(app) = self.app() else { return };
            let caret = self.caret.get();
            match app.split_block(caret) {
                Ok(()) => self.move_caret(
                    Caret {
                        block: caret.block + 1,
                        offset: 0,
                    },
                    true,
                ),
                Err(error) => self.notice(error.to_string()),
            }
        }

        /// Backspace: the character before the caret, and at the front of a block the
        /// boundary itself — which is what [`App::erase`] across one already does.
        pub fn erase_back(&self) {
            let Some(app) = self.app() else { return };
            let caret = self.caret.get();
            let from = self.stepped(&app, -1);
            if from == caret {
                return;
            }
            match app.erase(from, caret) {
                Ok(_) => self.move_caret(from, true),
                Err(error) => self.notice(error.to_string()),
            }
        }

        pub fn erase_forward(&self) {
            let Some(app) = self.app() else { return };
            let caret = self.caret.get();
            let to = match caret.offset < self.block_len(&app, caret.block) {
                true => Caret {
                    block: caret.block,
                    offset: caret.offset + 1,
                },
                false if caret.block + 1 < app.block_count() => Caret {
                    block: caret.block + 1,
                    offset: 0,
                },
                false => return,
            };
            if let Err(error) = app.erase(caret, to) {
                self.notice(error.to_string());
            }
        }

        // --- the caret ---

        /// The one place the caret changes: keep it inside the document, scroll it into
        /// view, tell the listeners, repaint.
        pub fn move_caret(&self, caret: Caret, clear_goal: bool) {
            self.caret.set(caret);
            if clear_goal {
                self.goal_x.set(None);
            }
            self.clamp_caret();
            self.scroll_into_view();
            self.announce();
            self.obj().queue_draw();
            let caret = self.caret.get();
            for hook in self.on_moved.borrow().iter() {
                hook(caret);
            }
        }

        /// History and edits move blocks around underneath the caret, so put it somewhere
        /// that exists.
        pub fn clamp_caret(&self) {
            let Some(app) = self.app() else { return };
            let mut caret = self.caret.get();
            caret.block = caret.block.min(app.block_count().saturating_sub(1));
            caret.offset = caret.offset.min(self.block_len(&app, caret.block));
            self.caret.set(caret);
        }

        fn scroll_into_view(&self) {
            let widget = self.obj();
            // Before the first allocation there is no view to scroll into.
            if widget.width() == 0 || widget.height() == 0 {
                return;
            }
            let flow = self.flow(f64::from(widget.width()));
            let caret = self.caret.get();
            let Some(slot) = flow.slot(caret.block) else {
                return;
            };
            let Some((layout, _, _)) = self.measured(caret.block) else {
                return;
            };
            let line = layout.lines()[layout.line_at(caret.offset)];
            let target = (slot.top + f64::from(line.top), f64::from(line.height));
            let Some(adjustment) = self.vadjustment.borrow().clone() else {
                return;
            };
            let page = f64::from(widget.height());
            adjustment.set_value(flow.follow(adjustment.value(), page, target));
        }

        /// The a11y floor (`doc/gtk-shell.md`, M9): a custom-drawn document has no other way
        /// to tell assistive technology that the caret moved, so every move speaks the
        /// block's address and its text.
        fn announce(&self) {
            let Some(app) = self.app() else { return };
            let caret = self.caret.get();
            let address = loc::format_offset(caret.block, caret.offset);
            let message = match app.input_text(caret.block) {
                Ok(text) if !text.is_empty() => format!("{address}: {text}"),
                _ => address,
            };
            self.obj()
                .announce(&message, gtk::AccessibleAnnouncementPriority::Medium);
        }

        fn notice(&self, message: String) {
            for hook in self.on_notice.borrow().iter() {
                hook(message.clone());
            }
        }
    }

    /// How far a block's text is indented — a list's nesting, and nothing else.
    fn indent_of(kind: &BlockKind) -> f64 {
        match kind {
            BlockKind::ListItem { depth } => f64::from(*depth) * geom::INDENT,
            _ => 0.0,
        }
    }

    /// Which line of a layout a y coordinate is on, measured from the block's own top.
    fn line_at_y(layout: &Layout, y: f64) -> usize {
        let last = layout.lines().len().saturating_sub(1);
        layout
            .lines()
            .iter()
            .position(|line| y < f64::from(line.top + line.height))
            .unwrap_or(last)
    }

    fn draw_at(
        snapshot: &gtk::Snapshot,
        layout: &pango::Layout,
        x: f64,
        y: f64,
        color: gtk::gdk::RGBA,
    ) {
        snapshot.save();
        snapshot.translate(&graphene::Point::new(x as f32, y as f32));
        snapshot.append_layout(layout, &color);
        snapshot.restore();
    }

    fn rect(x: f64, y: f64, w: f64, h: f64) -> graphene::Rect {
        graphene::Rect::new(x as f32, y as f32, w as f32, h as f32)
    }

    /// Size the scrollbar. Unlike a spreadsheet's, a document has an end, so `upper` is
    /// simply how tall it is.
    fn configure(adjustment: Option<&gtk::Adjustment>, page: f64, height: f64, step: f64) {
        let Some(adjustment) = adjustment else { return };
        let upper = height.max(page);
        adjustment.configure(
            adjustment.value().clamp(0.0, (upper - page).max(0.0)),
            0.0,
            upper,
            // A wheel notch is three lines, which is what every other document view does.
            step * 3.0,
            (page - step).max(step),
            page,
        );
    }

    /// A GDK keyval as [`keymap`] spells it. The keypad duplicates matter: a numeric-keypad
    /// arrow with Num Lock off is a different keyval and the same intent.
    fn key_of(keyval: gtk::gdk::Key) -> Key {
        use gtk::gdk::Key as K;
        match keyval {
            K::Left | K::KP_Left => Key::Left,
            K::Right | K::KP_Right => Key::Right,
            K::Up | K::KP_Up => Key::Up,
            K::Down | K::KP_Down => Key::Down,
            K::Home | K::KP_Home => Key::Home,
            K::End | K::KP_End => Key::End,
            K::Page_Up | K::KP_Page_Up => Key::PageUp,
            K::Page_Down | K::KP_Page_Down => Key::PageDown,
            K::Return | K::KP_Enter => Key::Return,
            K::BackSpace => Key::Backspace,
            K::Delete | K::KP_Delete => Key::Delete,
            _ => Key::Other,
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use grind_core::layout::{Fixed, Fragment, wrap};
        use grind_core::style::TextStyle;

        /// `Layout` is the core's, so this checks only the shell's own reading of it: which
        /// line a y coordinate falls on, including above and below the block.
        #[test]
        fn a_y_coordinate_picks_the_line_it_is_inside() {
            let style = TextStyle::default();
            let layout = wrap(
                &[Fragment {
                    text: "the cat sat on the mat",
                    style: &style,
                }],
                10.0,
                &Fixed,
            );
            assert!(layout.lines().len() > 1, "the fixture has to wrap");
            assert_eq!(line_at_y(&layout, 0.0), 0);
            assert_eq!(line_at_y(&layout, 1.5), 1, "one unit per line under Fixed");
            assert_eq!(line_at_y(&layout, -50.0), 0, "above the block");
            assert_eq!(
                line_at_y(&layout, 500.0),
                layout.lines().len() - 1,
                "below it"
            );
        }

        #[test]
        fn only_a_list_item_is_indented_and_it_is_by_its_depth() {
            assert_eq!(indent_of(&BlockKind::Paragraph), 0.0);
            assert_eq!(indent_of(&BlockKind::Heading { level: 1 }), 0.0);
            assert_eq!(
                indent_of(&BlockKind::ListItem { depth: 2 }),
                2.0 * geom::INDENT
            );
        }
    }
}

/// Resolve an address a user typed — `p12`, `#intro`, `§2.1.3` — against the document as it
/// now is.
///
/// The addressing no word processor's UI offers, and the reason this shell has a "Go to"
/// entry at all: `#intro` and `§2.1` survive edits above them where a line number does not.
pub fn caret_of(app: &App, address: &str) -> Result<Caret, String> {
    let loc = grind_text::loc::parse(address).map_err(|error| error.to_string())?;
    app.resolve_caret(&loc).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use grind_text::BlockKind;

    /// A widget with a document in it and a size to lay it out at — **or `None` where there
    /// is no display**, which is where CI runs (`.github/workflows/gtk.yml` installs the GTK
    /// packages and no compositor, on purpose).
    ///
    /// Everything decidable without a display is in `geom.rs` and `keymap.rs` and is tested
    /// unconditionally. What needs one is the part that asks Pango to measure, so this skips
    /// with a notice rather than pretending — the same rule the corpus loops follow.
    fn shell(paragraphs: &[&str]) -> Option<(Doc, Arc<App>)> {
        if gtk::init().is_err() {
            eprintln!("no display — skipping the widget tests");
            return None;
        }
        let app = Arc::new(App::new());
        for (index, text) in paragraphs.iter().enumerate() {
            app.insert(index, BlockKind::Paragraph, text)
                .expect("inserts");
        }
        let doc = Doc::new(app.clone());
        // Unparented widgets have no size, and a caret motion measured at a width of zero
        // wraps every paragraph one character to the line.
        doc.allocate(600, 400, -1, None);
        Some((doc, app))
    }

    fn text(app: &App) -> String {
        app.get_viewport(0..app.block_count())
            .iter()
            .map(|block| block.text.clone())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The shell's whole editing surface, driven the way the input method and the key
    /// controller drive it. What this proves is the wiring: every one of these is a core
    /// call, and getting the caret wrong afterwards is this file's own bug.
    #[test]
    fn typing_enter_and_backspace_reach_the_document() {
        let Some((doc, app)) = shell(&["hello world"]) else {
            return;
        };
        let imp = doc.imp();
        imp.move_caret(
            Caret {
                block: 0,
                offset: 5,
            },
            true,
        );
        imp.type_text(" there");
        assert_eq!(text(&app), "hello there world");
        assert_eq!(doc.caret().offset, 11, "the caret follows what was typed");

        imp.split();
        assert_eq!(text(&app), "hello there\n world");
        assert_eq!(
            doc.caret(),
            Caret {
                block: 1,
                offset: 0
            }
        );

        // Backspace at the front of a block joins it back onto the one above.
        imp.erase_back();
        assert_eq!(text(&app), "hello there world");
        assert_eq!(doc.caret().offset, 11);

        // And undo takes the three of them back out, in the core — this shell has no
        // history of its own and never will (doc/plan.md rule 2).
        for _ in 0..3 {
            assert!(app.undo());
        }
        assert_eq!(text(&app), "hello world");
    }

    /// The test S9 exists for, and the GTK half of `doc/text-layout.md`'s payoff: Down is
    /// not "the next block", it is the next *line*, and the answer comes from the core —
    /// measured in pixels through Pango here and in cells in the terminal.
    #[test]
    fn down_moves_by_a_wrapped_line_not_by_a_block() {
        let long = "the cat sat on the mat and then it slept for a very long time indeed \
                    while the rain kept on falling outside the window all afternoon";
        let Some((doc, app)) = shell(&[long, "after"]) else {
            return;
        };
        let imp = doc.imp();
        let (layout, faces, kind) = imp.measured(0).expect("the block lays out");
        assert!(
            layout.lines().len() > 1,
            "the fixture has to actually wrap or this proves nothing"
        );
        drop((faces, kind));

        imp.go(crate::keymap::Motion::Line(1));
        assert_eq!(doc.caret().block, 0, "still inside the same paragraph");
        assert!(doc.caret().offset > 0, "but further down it");

        // Off the end of the last line, the same key carries into the next block — a
        // document is one flow, not a list of boxes.
        for _ in 0..layout.lines().len() {
            imp.go(crate::keymap::Motion::Line(1));
        }
        assert_eq!(doc.caret().block, 1);
        assert_eq!(app.block_count(), 2, "and nothing was edited on the way");
    }

    #[test]
    fn an_address_resolves_to_a_caret_and_a_bad_one_says_so() {
        let app = App::new();
        app.insert(0, BlockKind::Paragraph, "hello")
            .expect("inserts");
        app.insert(1, BlockKind::Paragraph, "there")
            .expect("inserts");
        assert_eq!(
            caret_of(&app, "p2+3"),
            Ok(Caret {
                block: 1,
                offset: 3
            })
        );
        assert!(caret_of(&app, "nowhere").is_err());
    }
}
