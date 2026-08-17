// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The grid: a custom widget that draws a rectangle of cells and nothing else.
//!
//! It owns no data. Every paint asks [`App::get_viewport`] for exactly the cells that fit
//! on screen and throws them away again — which is what makes a 1048576 × 16384 sheet cost
//! a screenful (doc/plan.md rule 1).
//!
//! **Not `GtkColumnView`.** That widget is row-oriented, wants to own a list model, has no
//! rectangular selection and does not virtualise 16384 columns — and a widget that owns
//! the document is rule 1's trap in its nastiest form. A `GtkWidget` implementing
//! `GtkScrollable` and drawing in `snapshot()` is more code once and less argument forever.
//!
//! Layout arithmetic lives in [`crate::geom`] and colours in [`crate::theme`], so what is
//! left here is the drawing itself. Two things about that are worth knowing:
//!
//! * **GTK 4 has no partial invalidation.** There is no `queue_draw_area`, so every change
//!   redraws the widget — which is fine, because the cost is bounded by the cells that fit
//!   on screen rather than by the sheet.
//! * **Text overflows into empty neighbours, and numbers do not.** That asymmetry is the
//!   spreadsheet convention every user expects: a label wider than its column keeps going
//!   until it meets something, and a number that will not fit becomes `###` rather than
//!   silently showing the wrong magnitude. See [`imp::Grid::draw_cells`].
//!
//! Selection lives here rather than in the core because it is presentation state: an anchor
//! and an active cell, and [`crate::keymap`] holds every rule about moving them. This widget
//! translates events into that vocabulary, applies the answer, and repaints.

use std::cell::{Cell, RefCell};
use std::sync::Arc;

use libadwaita::gtk;
use libadwaita::prelude::*;

use gtk::glib;
use libadwaita::subclass::prelude::ObjectSubclassIsExt;
use sheet_core::App;

use crate::geom::{ColWidths, GridGeom, MAX_COLS, MAX_ROWS};
use crate::keymap::Selection;

glib::wrapper! {
    pub struct Grid(ObjectSubclass<imp::Grid>)
        @extends gtk::Widget,
        @implements gtk::Scrollable, gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Grid {
    pub fn new(app: Arc<App>) -> Self {
        let grid: Self = glib::Object::builder().build();
        grid.imp().app.replace(Some(app));
        grid
    }

    /// Which sheet is drawn. Sheet switching is a later milestone; this is how it will
    /// arrive.
    pub fn set_sheet(&self, sheet: usize) {
        self.imp().sheet.set(sheet);
        self.set_selection(Selection::default());
        self.queue_draw();
    }

    pub fn selection(&self) -> Selection {
        self.imp().selection.get()
    }

    pub fn sheet(&self) -> usize {
        self.imp().sheet.get()
    }

    /// Move the selection from outside the widget — what the name box navigates with, and
    /// what primes the status bar before anything has been clicked.
    pub fn set_selection(&self, selection: Selection) {
        self.imp().set_selection(selection);
    }

    /// The text being edited, which the formula bar shares.
    ///
    /// **One buffer, two views.** The in-cell editor and the formula bar are both
    /// `GtkEditable`s over this, so their content stays in step for free while each keeps
    /// its own caret and selection — which is exactly the familiar behaviour. When nothing
    /// is being edited it holds the active cell's `App::input_text`, which is what makes
    /// the formula bar show the cell.
    pub fn buffer(&self) -> gtk::EntryBuffer {
        self.imp().buffer.clone()
    }

    /// Start editing the active cell from outside — the formula bar taking focus.
    pub fn begin_edit(&self, focus_cell: bool) {
        self.imp().begin(crate::state::Seed::Cell, focus_cell);
    }

    /// Store what the buffer holds and move on; the formula bar's Enter key.
    pub fn commit(&self, direction: Option<crate::keymap::Dir>) {
        self.imp().commit(direction);
    }

    pub fn is_editing(&self) -> bool {
        self.imp().mode.get().is_editing()
    }

    /// Throw the edit away and put the cell back the way it was.
    pub fn cancel_edit(&self) {
        self.imp().cancel();
    }

    /// Told whenever an edit opens or closes — the formula bar's ✓/✗ buttons, and anything
    /// else that only makes sense mid-edit.
    pub fn connect_editing_changed(&self, f: impl Fn(bool) + 'static) {
        self.imp().on_editing.replace(Some(Box::new(f)));
    }

    /// Told when something happened that a user has to hear about. Chrome turns these into
    /// a toast or a banner; the grid does not know which.
    pub fn connect_notice(&self, f: impl Fn(Notice) + 'static) {
        self.imp().on_notice.replace(Some(Box::new(f)));
    }

    /// Called after every selection change, with the selection that resulted — what the
    /// status bar and the formula bar are driven from. The grid does not know what they
    /// are; it reports, and chrome decides.
    pub fn connect_selection_changed(&self, f: impl Fn(Selection) + 'static) {
        self.imp().on_selection.replace(Some(Box::new(f)));
    }
}

/// Something the user has to be told, from an edit they made.
#[derive(Clone, Debug)]
pub enum Notice {
    /// The formula would not parse, so nothing was stored. Carries the message and the byte
    /// offset of the problem.
    BadFormula(String, usize),
    /// The edit landed, and the recalculation behind it was skipped because it would have
    /// replaced this many cached values with errors.
    RecalcSkipped(usize),
}

impl Default for Grid {
    fn default() -> Self {
        glib::Object::builder().build()
    }
}

mod imp {
    use super::*;

    use gtk::graphene;
    use gtk::pango;
    use gtk::subclass::prelude::*;
    use sheet_core::formula::value::FormulaError;
    use sheet_core::{CellValue, Pos};

    use sheet_core::RecalcMode;
    use sheet_core::formula::display;

    use crate::geom::{Hit, Rect};
    use crate::keymap::{self, Action, Dir, Extent, Key, Mods};
    use crate::state::{self, Mode, Outcome, Seed};
    use crate::theme::{Palette, with_alpha};

    use super::Notice;

    /// What the grid tells chrome when the selection moves.
    type SelectionHook = Box<dyn Fn(Selection)>;
    type NoticeHook = Box<dyn Fn(Notice)>;
    type EditingHook = Box<dyn Fn(bool)>;

    /// Space either side of a cell's text.
    const PAD: f64 = 4.0;
    /// How many columns are fetched beyond the visible ones, so that a label anchored just
    /// off-screen still overflows into view.
    const OVERFLOW_MARGIN: u32 = 12;

    /// Font-derived sizes. Recomputed whenever the style changes, because a theme with a
    /// bigger font needs taller rows, not clipped text.
    #[derive(Clone, Copy, Debug)]
    pub struct Metrics {
        pub row_height: f64,
        pub col_width: f64,
        pub header_w: f64,
        pub header_h: f64,
    }

    impl Default for Metrics {
        fn default() -> Self {
            Self {
                row_height: 24.0,
                col_width: 96.0,
                header_w: 56.0,
                header_h: 24.0,
            }
        }
    }

    /// One paint: where the view is, what colours it uses, and which cells fall inside
    /// it. Passed around rather than six arguments, because every drawing step needs the
    /// same six and a seventh will arrive with the selection.
    struct Frame<'a> {
        snapshot: &'a gtk::Snapshot,
        geom: GridGeom,
        palette: Palette,
        width: f64,
        height: f64,
        rows: std::ops::Range<u32>,
        cols: std::ops::Range<u32>,
        selection: Selection,
    }

    pub struct Grid {
        pub app: RefCell<Option<Arc<App>>>,
        pub sheet: Cell<usize>,
        pub hadjustment: RefCell<Option<gtk::Adjustment>>,
        pub vadjustment: RefCell<Option<gtk::Adjustment>>,
        pub hscroll_policy: Cell<gtk::ScrollablePolicy>,
        pub vscroll_policy: Cell<gtk::ScrollablePolicy>,
        pub metrics: Cell<Metrics>,
        pub palette: RefCell<Option<Palette>>,
        /// One reusable layout rather than one per cell.
        ///
        /// ponytail: `set_text` re-shapes on every cell, where a `(row, col)` cache would
        /// re-shape only what changed. This is the cheaper thing to write and allocates
        /// nothing per frame; add the cache when a profiler blames shaping rather than
        /// before.
        pub layout: RefCell<Option<pango::Layout>>,
        /// Presentation state, and the only state this widget has.
        pub selection: Cell<Selection>,
        /// What a drag started on, so that dragging across headers selects whole columns
        /// rather than the cells the pointer happens to pass over.
        pub drag: Cell<Option<Hit>>,
        pub on_selection: RefCell<Option<SelectionHook>>,
        pub on_notice: RefCell<Option<NoticeHook>>,
        pub on_editing: RefCell<Option<EditingHook>>,
        /// Ready, or an edit in progress. The whole editing state, since the text lives in
        /// the buffer and the cell in the selection.
        pub mode: Cell<Mode>,
        /// The in-cell editor: a real `gtk::Text` child, allocated over the active cell.
        ///
        /// Not a custom-drawn one, and not negotiable: `gtk::Text` brings the input method
        /// with it — preedit, dead keys, CJK, Compose — plus the caret, selection and
        /// clipboard. Hand-rolling `GtkIMContext` is the classic way to ship broken input.
        pub editor: gtk::Text,
        pub buffer: gtk::EntryBuffer,
        /// Where the editor was last put, so that a click inside it can be told from a
        /// click on the sheet. `WidgetExt::allocation` would answer the same question and
        /// is deprecated as of 4.12.
        pub editor_rect: Cell<Rect>,
    }

    impl Default for Grid {
        fn default() -> Self {
            Self {
                app: RefCell::new(None),
                sheet: Cell::new(0),
                hadjustment: RefCell::new(None),
                vadjustment: RefCell::new(None),
                hscroll_policy: Cell::new(gtk::ScrollablePolicy::Minimum),
                vscroll_policy: Cell::new(gtk::ScrollablePolicy::Minimum),
                metrics: Cell::new(Metrics::default()),
                palette: RefCell::new(None),
                layout: RefCell::new(None),
                selection: Cell::new(Selection::default()),
                drag: Cell::new(None),
                on_selection: RefCell::new(None),
                on_notice: RefCell::new(None),
                on_editing: RefCell::new(None),
                mode: Cell::new(Mode::default()),
                editor: gtk::Text::new(),
                buffer: gtk::EntryBuffer::default(),
                editor_rect: Cell::new(Rect {
                    x: 0.0,
                    y: 0.0,
                    w: 0.0,
                    h: 0.0,
                }),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Grid {
        const NAME: &'static str = "SheetGrid";
        type Type = super::Grid;
        type ParentType = gtk::Widget;
        type Interfaces = (gtk::Scrollable,);

        fn class_init(klass: &mut Self::Class) {
            klass.set_css_name("sheetgrid");
            klass.set_accessible_role(gtk::AccessibleRole::Grid);
        }
    }

    impl ObjectImpl for Grid {
        // The four properties `GtkScrollable` requires. Overridden by hand rather than by
        // the `Properties` derive, whose `override_interface` spelling has churned between
        // gtk4-rs releases; this shape does not move.
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
                "hadjustment" => self.set_adjustment(gtk::Orientation::Horizontal, value.get().ok()),
                "vadjustment" => self.set_adjustment(gtk::Orientation::Vertical, value.get().ok()),
                "hscroll-policy" => self.hscroll_policy.set(value.get().unwrap()),
                "vscroll-policy" => self.vscroll_policy.set(value.get().unwrap()),
                other => unimplemented!("property {other}"),
            }
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

        /// A widget with children has to unparent them, or GTK complains at teardown.
        fn dispose(&self) {
            self.editor.unparent();
        }

        fn constructed(&self) {
            self.parent_constructed();
            let widget = self.obj();
            widget.set_focusable(true);

            // The editor is a child of the grid rather than an overlay: an overlay
            // positions in widget coordinates and would re-derive the scroll arithmetic
            // every frame, where a child is allocated by the same `cell_rect` that draws.
            self.editor.set_buffer(&self.buffer);
            self.editor.set_parent(&*widget);
            self.editor.set_visible(false);
            self.editor.add_css_class("sheet-editor");

            // **Capture phase**, so the grid decides before the editor child does. Every
            // key it does not claim then travels on to the editor untouched, which is what
            // keeps the input method working.
            let keys = gtk::EventControllerKey::new();
            keys.set_propagation_phase(gtk::PropagationPhase::Capture);
            keys.connect_key_pressed(glib::clone!(
                #[weak(rename_to = grid)]
                widget,
                #[upgrade_or]
                glib::Propagation::Proceed,
                move |_, keyval, _, state| grid.imp().key_pressed(keyval, state)
            ));
            widget.add_controller(keys);

            // One gesture for click and drag alike: a click is a drag that never moved, and
            // writing them separately is how the two disagree about what shift means.
            let click = gtk::GestureClick::new();
            click.connect_pressed(glib::clone!(
                #[weak(rename_to = grid)]
                widget,
                move |_, presses, _, _| {
                    if presses == 2 {
                        grid.imp().begin(Seed::Cell, true);
                    }
                }
            ));
            widget.add_controller(click);

            let drag = gtk::GestureDrag::new();
            drag.connect_drag_begin(glib::clone!(
                #[weak(rename_to = grid)]
                widget,
                move |gesture, x, y| {
                    grid.grab_focus();
                    let extend = gesture
                        .current_event_state()
                        .contains(gtk::gdk::ModifierType::SHIFT_MASK);
                    grid.imp().press(x, y, extend);
                }
            ));
            drag.connect_drag_update(glib::clone!(
                #[weak(rename_to = grid)]
                widget,
                move |gesture, dx, dy| {
                    if let Some((x, y)) = gesture.start_point() {
                        grid.imp().extend_to(x + dx, y + dy);
                    }
                }
            ));
            drag.connect_drag_end(glib::clone!(
                #[weak(rename_to = grid)]
                widget,
                move |_, _, _| grid.imp().drag.set(None)
            ));
            widget.add_controller(drag);
        }
    }

    impl ScrollableImpl for Grid {}

    impl Grid {
        /// Where the in-cell editor sits: over the active cell, grown to fit the text but
        /// never past the edge of the view.
        fn allocate_editor(&self, width: f64) {
            if !self.editor.is_visible() {
                return;
            }
            let active = self.selection.get().active;
            let cell = self.geom().cell_rect(active.row, active.col);
            let (_, natural, _, _) = self.editor.measure(gtk::Orientation::Horizontal, -1);
            let w = f64::from(natural)
                .max(cell.w)
                .min((width - cell.x).max(cell.w));
            let rect = Rect { w, ..cell };
            self.editor_rect.set(rect);
            self.editor.size_allocate(
                &gtk::gdk::Rectangle::new(rect.x as i32, rect.y as i32, rect.w as i32, rect.h as i32),
                -1,
            );
        }
    }

    impl WidgetImpl for Grid {
        /// A scrollable asks for nothing and takes what it is given; the scrolled window
        /// decides the size and this decides what fits in it.
        fn measure(&self, _orientation: gtk::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
            (0, 0, -1, -1)
        }

        fn size_allocate(&self, width: i32, height: i32, _baseline: i32) {
            self.configure_adjustments(f64::from(width), f64::from(height));
            self.allocate_editor(f64::from(width));
        }

        /// The font and the metrics derived from it are only knowable once the widget has
        /// a display to ask.
        fn realize(&self) {
            self.parent_realize();
            self.restyle();
        }

        /// A theme switch, a font change or a new display: everything derived from the
        /// style is dropped and derived again.
        fn system_setting_changed(&self, setting: &gtk::SystemSetting) {
            self.parent_system_setting_changed(setting);
            self.restyle();
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let widget = self.obj();
            let width = f64::from(widget.width());
            let height = f64::from(widget.height());
            let geom = self.geom();
            let frame = Frame {
                snapshot,
                palette: self.palette(),
                rows: geom.visible_rows(height),
                cols: geom.visible_cols(width),
                geom,
                width,
                height,
                selection: self.selection.get(),
            };

            snapshot.append_color(&frame.palette.background, &rect(0.0, 0.0, width, height));

            snapshot.push_clip(&rect(
                geom.header_w,
                geom.header_h,
                width - geom.header_w,
                height - geom.header_h,
            ));
            self.draw_selection(&frame);
            self.draw_lines(&frame);
            self.draw_cells(&frame);
            self.draw_active(&frame);
            snapshot.pop();

            self.draw_headers(&frame);
            // Last, and only while editing: a real child widget, drawn over its cell.
            if self.editor.is_visible() {
                widget.snapshot_child(&self.editor, snapshot);
            }
        }
    }

    impl Grid {
        fn geom(&self) -> GridGeom {
            let m = self.metrics.get();
            GridGeom {
                header_w: m.header_w,
                header_h: m.header_h,
                row_height: m.row_height,
                cols: ColWidths::Uniform(m.col_width),
                scroll_x: self
                    .hadjustment
                    .borrow()
                    .as_ref()
                    .map_or(0.0, gtk::Adjustment::value),
                scroll_y: self
                    .vadjustment
                    .borrow()
                    .as_ref()
                    .map_or(0.0, gtk::Adjustment::value),
            }
        }

        fn palette(&self) -> Palette {
            if let Some(palette) = *self.palette.borrow() {
                return palette;
            }
            let palette = Palette::of(&*self.obj());
            self.palette.replace(Some(palette));
            palette
        }

        fn layout(&self) -> pango::Layout {
            if let Some(layout) = self.layout.borrow().as_ref() {
                return layout.clone();
            }
            let layout = self.obj().create_pango_layout(None);
            self.layout.replace(Some(layout.clone()));
            layout
        }

        /// Drop everything derived from the style and derive it again.
        fn restyle(&self) {
            self.palette.replace(None);
            self.layout.replace(None);
            self.update_metrics();
            self.obj().queue_resize();
        }

        /// Row height, column width and the header band, all derived from the font — so a
        /// larger text size gives taller rows rather than clipped text.
        fn update_metrics(&self) {
            let metrics = self.obj().pango_context().metrics(None, None);
            let scale = f64::from(pango::SCALE);
            let line = f64::from(metrics.ascent() + metrics.descent()) / scale;
            let digit = (f64::from(metrics.approximate_digit_width()) / scale).max(1.0);
            let row_height = (line + 8.0).ceil();
            self.metrics.set(Metrics {
                row_height,
                col_width: (digit * 11.0).ceil(),
                // Wide enough for the last row number, 1048576.
                header_w: (digit * 8.0).ceil(),
                header_h: row_height,
            });
        }

        fn set_adjustment(&self, orientation: gtk::Orientation, adjustment: Option<gtk::Adjustment>) {
            let adjustment = adjustment.unwrap_or_default();
            adjustment.connect_value_changed(glib::clone!(
                #[weak(rename_to = grid)]
                self.obj(),
                move |_| {
                    // The editor is allocated over a cell, so scrolling has to move it as
                    // well as repaint — otherwise it stays put while the sheet slides.
                    if grid.imp().editor.is_visible() {
                        grid.queue_allocate();
                    }
                    grid.queue_draw();
                }
            ));
            match orientation {
                gtk::Orientation::Horizontal => self.hadjustment.replace(Some(adjustment)),
                _ => self.vadjustment.replace(Some(adjustment)),
            };
            self.obj().queue_allocate();
        }

        /// Size the scrollbars.
        ///
        /// `upper` is deliberately **not** the sheet's full extent: a thumb sized against
        /// 1048576 rows is a few pixels tall and one click lands in row 800000. It is the
        /// used extent plus a screenful — enough to scroll into blank space, which is
        /// normal — and it grows as the view moves into it.
        fn configure_adjustments(&self, width: f64, height: f64) {
            let geom = self.geom();
            let (used_rows, used_cols) = self.used_extent();
            let page_w = (width - geom.header_w).max(1.0);
            let page_h = (height - geom.header_h).max(1.0);

            configure(
                self.hadjustment.borrow().as_ref(),
                page_w,
                geom.cols.x_of(used_cols.min(MAX_COLS)),
                geom.cols.width_of(0),
                geom.cols.total(),
            );
            configure(
                self.vadjustment.borrow().as_ref(),
                page_h,
                geom.y_of(used_rows.min(MAX_ROWS)),
                geom.row_height,
                geom.y_of(MAX_ROWS),
            );
        }

        fn used_extent(&self) -> (u32, u32) {
            self.app
                .borrow()
                .as_ref()
                .and_then(|app| app.used_extent(self.sheet.get()).ok())
                .unwrap_or((0, 0))
        }

        // --- events ---

        /// A key, in [`keymap`]'s vocabulary. `Proceed` for everything this shell does not
        /// claim, which is what keeps the toolkit's own bindings working.
        fn key_pressed(&self, keyval: gtk::gdk::Key, state: gtk::gdk::ModifierType) -> glib::Propagation {
            let mods = Mods {
                ctrl: state.contains(gtk::gdk::ModifierType::CONTROL_MASK),
                shift: state.contains(gtk::gdk::ModifierType::SHIFT_MASK),
                alt: state.contains(gtk::gdk::ModifierType::ALT_MASK),
            };
            let action = match state::on_key(self.mode.get(), key_of(keyval), mods) {
                Outcome::Passthrough => return glib::Propagation::Proceed,
                Outcome::Navigate(action) => action,
                Outcome::Begin(seed) => {
                    self.begin(seed, true);
                    // The seeded character is already in the buffer, so the key must not
                    // also reach the editor and be typed a second time.
                    return glib::Propagation::Stop;
                }
                Outcome::Commit(direction) => {
                    self.commit(direction);
                    return glib::Propagation::Stop;
                }
                Outcome::Cancel => {
                    self.cancel();
                    return glib::Propagation::Stop;
                }
                Outcome::Clear => {
                    self.clear();
                    return glib::Propagation::Stop;
                }
                Outcome::ToggleMode => {
                    self.mode.set(match self.mode.get() {
                        Mode::Edit => Mode::Enter,
                        _ => Mode::Edit,
                    });
                    return glib::Propagation::Stop;
                }
            };
            let selection = match action {
                Action::Move { motion, extend } => {
                    let app = self.app.borrow().clone();
                    let sheet = self.sheet.get();
                    // One cell read per probe. ponytail: a scan across a sparse million-row
                    // sheet is a million point reads; the fix is a used-extent walk in the
                    // core, not a cache here.
                    let occupied = |pos: Pos| {
                        app.as_ref()
                            .and_then(|app| app.get(sheet, pos).ok())
                            .is_some_and(|value| !value.is_empty())
                    };
                    keymap::moved(self.selection.get(), motion, extend, self.extent(), &occupied)
                }
                Action::SelectAll => {
                    let (rows, cols) = self.used_extent();
                    Selection {
                        anchor: Pos::new(0, 0),
                        active: Pos::new(rows.saturating_sub(1), cols.saturating_sub(1)),
                    }
                }
                Action::Copy => {
                    self.copy(false);
                    return glib::Propagation::Stop;
                }
                Action::Cut => {
                    self.copy(true);
                    return glib::Propagation::Stop;
                }
                Action::Paste => {
                    self.paste();
                    return glib::Propagation::Stop;
                }
            };
            self.set_selection(selection);
            glib::Propagation::Stop
        }

        /// Start editing the active cell.
        ///
        /// `Seed::Cell` needs no work: the buffer already holds the cell's input text,
        /// because that is what it holds whenever nothing is being edited.
        pub fn begin(&self, seed: Seed, focus_cell: bool) {
            if let Seed::Char(c) = seed {
                self.buffer.set_text(c.to_string());
            }
            self.mode.set(match seed {
                Seed::Char(_) => Mode::Enter,
                Seed::Cell => Mode::Edit,
            });
            self.editor.set_visible(true);
            self.editing_changed(true);
            if focus_cell {
                self.editor.grab_focus();
                // Focusing selects everything, which would make the next keystroke delete
                // the seed; the caret belongs at the end of what is there.
                self.editor.set_position(-1);
            }
            self.obj().queue_allocate();
            self.obj().queue_draw();
        }

        /// Store what the buffer holds, then move.
        ///
        /// Display form goes back to canonical here — the one step between what an editor
        /// holds and what `App::enter` takes — and a formula that will not parse **does not
        /// commit**: the edit stays open with the caret on the problem, because silently
        /// storing `=SUM(B2` as text is how a spreadsheet loses a user's work.
        pub fn commit(&self, direction: Option<Dir>) {
            let app = self.app.borrow().clone();
            let (Some(app), true) = (app, self.mode.get().is_editing()) else {
                return;
            };
            let text = self.buffer.text().to_string();
            // Nothing typed, nothing stored: opening a cell and closing it again must not
            // push an undo entry or mark the document modified.
            let active = self.selection.get().active;
            if app
                .input_text(self.sheet.get(), active)
                .is_ok_and(|before| before == text)
            {
                self.end_edit();
                self.move_after_commit(active, direction);
                return;
            }
            let input = match text.starts_with('=') {
                true => match display::from_display(&text) {
                    Ok(canonical) => canonical,
                    Err(e) => {
                        self.editor.set_position(caret_at(&text, e.at));
                        self.notice(Notice::BadFormula(e.message, e.at));
                        return;
                    }
                },
                false => text,
            };

            // `RecalcMode::Document` is what makes a GUI feel live: the ripple lands in the
            // same undo entry. It is skipped rather than refused when it would spoil a
            // cached value, and the notice is how that gets said out loud.
            match app.enter(self.sheet.get(), active, &input, RecalcMode::Document) {
                Ok(outcome) => {
                    if let Some(recalc) = outcome.recalc.filter(|r| r.spoiled > 0) {
                        self.notice(Notice::RecalcSkipped(recalc.spoiled));
                    }
                }
                Err(error) => self.notice(Notice::BadFormula(error.to_string(), 0)),
            }
            self.end_edit();
            self.move_after_commit(active, direction);
        }

        /// Where the cursor goes once an edit is stored. Committing moves the *cursor*,
        /// never extends a selection.
        fn move_after_commit(&self, from: Pos, direction: Option<Dir>) {
            self.set_selection(match direction {
                Some(dir) => keymap::moved(
                    Selection::at(from),
                    keymap::Motion::By(dir),
                    false,
                    self.extent(),
                    &|_| false,
                ),
                None => Selection::at(from),
            });
        }

        /// Throw the edit away. The document is never touched, so there is nothing to undo.
        pub fn cancel(&self) {
            self.end_edit();
            self.refresh_buffer();
            self.obj().grab_focus();
            self.obj().queue_draw();
        }

        fn end_edit(&self) {
            self.mode.set(Mode::Ready);
            self.editor.set_visible(false);
            self.editing_changed(false);
            self.obj().grab_focus();
        }

        fn editing_changed(&self, editing: bool) {
            if let Some(hook) = self.on_editing.borrow().as_ref() {
                hook(editing);
            }
        }

        /// Put the active cell's editable text in the shared buffer — what the formula bar
        /// shows, and what F2 starts from.
        fn refresh_buffer(&self) {
            let app = self.app.borrow().clone();
            let text = app
                .and_then(|app| app.input_text(self.sheet.get(), self.selection.get().active).ok())
                .unwrap_or_default();
            self.buffer.set_text(text);
        }

        fn notice(&self, notice: Notice) {
            if let Some(hook) = self.on_notice.borrow().as_ref() {
                hook(notice);
            }
        }

        /// Put the selection on the clipboard as tab-separated rows, and with `cut`, empty
        /// it afterwards.
        ///
        /// What travels is each cell's `App::input_text` — the raw number, or a formula in
        /// display form — rather than what the cell *displays*. Two reasons: pasted back
        /// here it reproduces the cells exactly, and pasted into another spreadsheet
        /// `1234.5` is a number where `1,234.50 €` is a guess about that program's locale.
        ///
        /// ponytail: a cell holding a tab or a newline has them replaced with spaces, so
        /// that the rectangle survives. The upgrade is quoting, in a codec shared with
        /// `sheet paste` — both halves split on tabs today and neither may grow a private
        /// dialect (`doc/plan.md` rule 4).
        fn copy(&self, cut: bool) {
            let app = self.app.borrow().clone();
            let Some(app) = app else { return };
            let Some((start, end)) = self.clamped_selection() else {
                return;
            };
            let sheet = self.sheet.get();
            let text = (start.row..=end.row)
                .map(|row| {
                    (start.col..=end.col)
                        .map(|col| {
                            app.input_text(sheet, Pos::new(row, col))
                                .unwrap_or_default()
                                .replace(['\t', '\n', '\r'], " ")
                        })
                        .collect::<Vec<_>>()
                        .join("\t")
                })
                .collect::<Vec<_>>()
                .join("\n");
            self.obj().clipboard().set_text(&text);
            if cut {
                let _ = app.clear_range(sheet, start, end);
                self.obj().queue_draw();
            }
        }

        /// Read the clipboard and fill from the selection's top-left corner.
        ///
        /// Asynchronous because the clipboard is: the data may be owned by another process
        /// that has to be asked for it. Every cell goes through the same typing rule a
        /// keystroke does, in one `Action::Batch`, so a paste is one undo step.
        fn paste(&self) {
            let (start, _) = self.selection.get().rect();
            self.obj().clipboard().read_text_async(
                gtk::gio::Cancellable::NONE,
                glib::clone!(
                    #[weak(rename_to = grid)]
                    self.obj(),
                    move |result| {
                        let Ok(Some(text)) = result else { return };
                        let rows: Vec<Vec<String>> = text
                            .lines()
                            .map(|line| line.split('\t').map(str::to_owned).collect())
                            .collect();
                        let imp = grid.imp();
                        let app = imp.app.borrow().clone();
                        let Some(app) = app else { return };
                        match app.enter_range(imp.sheet.get(), start, &rows, RecalcMode::Document) {
                            Ok(outcome) => {
                                if let Some(recalc) = outcome.recalc.filter(|r| r.spoiled > 0) {
                                    imp.notice(Notice::RecalcSkipped(recalc.spoiled));
                                }
                                // Select what landed, which is what makes a paste visible
                                // and a second paste elsewhere obvious.
                                let last = Pos::new(
                                    start.row + rows.len().saturating_sub(1) as u32,
                                    start.col
                                        + rows.iter().map(Vec::len).max().unwrap_or(1).saturating_sub(1) as u32,
                                );
                                imp.set_selection(Selection {
                                    anchor: start,
                                    active: last,
                                });
                            }
                            Err(error) => imp.notice(Notice::BadFormula(error.to_string(), 0)),
                        }
                        grid.queue_draw();
                    }
                ),
            );
        }

        /// The selection, cut down to the cells that exist. A whole-column selection is
        /// 1048576 rows and every one of the operations here would otherwise be asked to
        /// walk them.
        fn clamped_selection(&self) -> Option<(Pos, Pos)> {
            let (start, end) = self.selection.get().rect();
            let (rows, cols) = self.used_extent();
            let end = Pos::new(
                end.row.min(rows.saturating_sub(1)),
                end.col.min(cols.saturating_sub(1)),
            );
            (rows > 0 && cols > 0 && end.row >= start.row && end.col >= start.col)
                .then_some((start, end))
        }

        /// Empty the selection — Delete, and one undo step whatever its size.
        fn clear(&self) {
            let app = self.app.borrow().clone();
            let (Some(app), Some((start, end))) = (app, self.clamped_selection()) else {
                return;
            };
            let _ = app.clear_range(self.sheet.get(), start, end);
            self.obj().queue_draw();
        }

        /// Press: a cell, or a whole column or row from its header.
        fn press(&self, x: f64, y: f64, extend: bool) {
            if self.mode.get().is_editing() {
                // A click *inside* the editor is a caret move, and the event reaches here
                // only because the editor is a child of this widget.
                if self.editor_rect.get().contains(x, y) {
                    return;
                }
                // Clicking away from an open edit stores it, which is what every
                // spreadsheet does and what a user who clicks the next cell means.
                self.commit(None);
            }
            let hit = self.geom().hit(x, y);
            self.drag.set(Some(hit));
            let Some(target) = self.selection_for(hit) else { return };
            self.set_selection(match extend {
                true => Selection {
                    anchor: self.selection.get().anchor,
                    active: target.active,
                },
                false => target,
            });
        }

        /// Drag: extend from the anchor, in whatever the press started on.
        fn extend_to(&self, x: f64, y: f64) {
            let Some(start) = self.drag.get() else { return };
            let hit = self.geom().hit(x, y);
            // A drag that began on a column header keeps selecting columns even when the
            // pointer wanders into the cells.
            let hit = match (start, hit) {
                (Hit::ColHeader(_) | Hit::ColEdge(_), Hit::Cell { col, .. }) => Hit::ColHeader(col),
                (Hit::RowHeader(_) | Hit::RowEdge(_), Hit::Cell { row, .. }) => Hit::RowHeader(row),
                _ => hit,
            };
            let Some(target) = self.selection_for(hit) else { return };
            self.set_selection(Selection {
                anchor: self.selection.get().anchor,
                active: target.active,
            });
        }

        /// What clicking on something selects. A header selects the whole column or row —
        /// the sheet's whole extent, not the used one, because that is what a user means by
        /// "this column" when they are about to type into it.
        fn selection_for(&self, hit: Hit) -> Option<Selection> {
            Some(match hit {
                Hit::Cell { row, col } => Selection::at(Pos::new(row, col)),
                Hit::ColHeader(col) | Hit::ColEdge(col) => Selection {
                    anchor: Pos::new(0, col),
                    active: Pos::new(MAX_ROWS - 1, col),
                },
                Hit::RowHeader(row) | Hit::RowEdge(row) => Selection {
                    anchor: Pos::new(row, 0),
                    active: Pos::new(row, MAX_COLS - 1),
                },
                Hit::Corner => Selection {
                    anchor: Pos::new(0, 0),
                    active: Pos::new(MAX_ROWS - 1, MAX_COLS - 1),
                },
            })
        }

        /// The one place a selection changes: scroll it into view, repaint, and tell
        /// whoever is listening.
        pub fn set_selection(&self, selection: Selection) {
            self.selection.set(selection);
            self.refresh_buffer();
            self.scroll_into_view(selection.active);
            self.obj().queue_draw();
            if let Some(hook) = self.on_selection.borrow().as_ref() {
                hook(selection);
            }
        }

        fn scroll_into_view(&self, pos: Pos) {
            // Before the first allocation there is no view to scroll into, and pretending
            // otherwise leaves the document opened halfway down its first screen.
            if self.obj().width() == 0 || self.obj().height() == 0 {
                return;
            }
            let geom = self.geom();
            let page_w = (f64::from(self.obj().width()) - geom.header_w).max(1.0);
            let page_h = (f64::from(self.obj().height()) - geom.header_h).max(1.0);
            let (x, y) = geom.scroll_into_view(pos.row, pos.col, page_w, page_h);
            // Setting a value the adjustment already has is a no-op, so this is also the
            // test for "did anything move".
            if let Some(h) = self.hadjustment.borrow().as_ref() {
                h.set_value(x.min((h.upper() - h.page_size()).max(0.0)));
            }
            if let Some(v) = self.vadjustment.borrow().as_ref() {
                v.set_value(y.min((v.upper() - v.page_size()).max(0.0)));
            }
        }

        /// The sheet's used extent plus how many rows a screenful is — what navigation
        /// needs to know about the document.
        fn extent(&self) -> Extent {
            let (rows, cols) = self.used_extent();
            let geom = self.geom();
            let visible = ((f64::from(self.obj().height()) - geom.header_h) / geom.row_height)
                .max(1.0) as u32;
            Extent {
                rows,
                cols,
                // A page keeps one row of context, which is what every other grid does.
                page: visible.saturating_sub(1).max(1),
            }
        }

        // --- drawing ---

        /// The selected rectangle, with the active cell left out of the wash so that it
        /// reads as the cell the cursor is in.
        fn draw_selection(&self, f: &Frame) {
            if f.selection.is_single() {
                return;
            }
            let (start, end) = f.selection.rect();
            let top_left = f.geom.cell_rect(start.row, start.col);
            let bottom_right = f.geom.cell_rect(end.row, end.col);
            // Clamped to the widget: a whole-column selection is 20 million pixels tall,
            // which is past the point where an f32 rectangle still lands on a pixel.
            let x = top_left.x.max(f.geom.header_w);
            let y = top_left.y.max(f.geom.header_h);
            let w = (bottom_right.x + bottom_right.w).min(f.width) - x;
            let h = (bottom_right.y + bottom_right.h).min(f.height) - y;
            if w <= 0.0 || h <= 0.0 {
                return;
            }
            f.snapshot
                .append_color(&with_alpha(f.palette.accent, 0.12), &rect(x, y, w, h));
            let active = f.geom.cell_rect(f.selection.active.row, f.selection.active.col);
            f.snapshot
                .append_color(&f.palette.background, &rect(active.x, active.y, active.w, active.h));
        }

        /// The active cell's border, drawn after the text so it is never painted over.
        fn draw_active(&self, f: &Frame) {
            let cell = f.geom.cell_rect(f.selection.active.row, f.selection.active.col);
            let t = 2.0;
            for edge in [
                rect(cell.x - 1.0, cell.y - 1.0, cell.w + 2.0, t),
                rect(cell.x - 1.0, cell.y + cell.h - 1.0, cell.w + 2.0, t),
                rect(cell.x - 1.0, cell.y - 1.0, t, cell.h + 2.0),
                rect(cell.x + cell.w - 1.0, cell.y - 1.0, t, cell.h + 2.0),
            ] {
                f.snapshot.append_color(&f.palette.accent, &edge);
            }
        }

        fn draw_lines(&self, f: &Frame) {
            for row in f.rows.clone() {
                let y = f.geom.cell_rect(row, 0).y;
                f.snapshot
                    .append_color(&f.palette.lines, &rect(f.geom.header_w, y, f.width, 1.0));
            }
            for col in f.cols.clone() {
                let x = f.geom.cell_rect(0, col).x;
                f.snapshot
                    .append_color(&f.palette.lines, &rect(x, f.geom.header_h, 1.0, f.height));
            }
        }

        /// Draw the values, with the two overflow rules.
        ///
        /// Columns are fetched with a margin either side so that a label anchored just
        /// off-screen still reaches into the view, and so that "is the next cell empty"
        /// can be answered without a second read.
        fn draw_cells(&self, f: &Frame) {
            let (geom, palette, rows, cols) = (&f.geom, &f.palette, &f.rows, &f.cols);
            let app = self.app.borrow();
            let Some(app) = app.as_ref() else { return };
            let fetch = cols.start.saturating_sub(OVERFLOW_MARGIN)
                ..(cols.end.saturating_add(OVERFLOW_MARGIN)).min(MAX_COLS);
            let Ok(viewport) = app.get_viewport(self.sheet.get(), rows.clone(), fetch.clone())
            else {
                return;
            };

            // The cell being edited is drawn by the editor child, not here — otherwise its
            // stored value shows through the text being typed over it.
            let editing = self.mode.get().is_editing().then(|| self.selection.get().active);
            let layout = self.layout();
            for row in rows.clone() {
                for col in fetch.clone() {
                    if editing == Some(Pos::new(row, col)) {
                        continue;
                    }
                    let Some(value) = viewport.get(row, col) else {
                        continue;
                    };
                    if value.is_empty() {
                        continue;
                    }
                    let text = viewport.text(row, col).unwrap_or_default();
                    if text.is_empty() {
                        continue;
                    }
                    layout.set_text(text);
                    let (text_w, text_h) = layout.pixel_size();
                    let cell = geom.cell_rect(row, col);
                    let fits = f64::from(text_w) <= cell.w - 2.0 * PAD;
                    let align = alignment(value);

                    // A number that does not fit is never truncated — a wrong magnitude
                    // read as a right one is worse than no reading at all.
                    if !fits && align == Align::Right {
                        layout.set_text("##########");
                        let (w, h) = layout.pixel_size();
                        draw_text(f.snapshot, &layout, palette.foreground, &cell, cell, w, h, Align::Right);
                        continue;
                    }

                    // Text keeps going until it meets something, which is the other half
                    // of the convention.
                    let mut paint = cell;
                    if !fits && align == Align::Left {
                        let stop = (col + 1..fetch.end)
                            .find(|c| viewport.get(row, *c).is_some_and(|v| !v.is_empty()))
                            .map_or(f.width + geom.scroll_x, |c| geom.cell_rect(row, c).x);
                        paint.w = (stop - cell.x).max(cell.w);
                    }
                    draw_text(
                        f.snapshot,
                        &layout,
                        palette.foreground,
                        &cell,
                        paint,
                        text_w,
                        text_h,
                        align,
                    );
                }
            }
        }

        fn draw_headers(&self, f: &Frame) {
            let (snapshot, geom, palette) = (f.snapshot, &f.geom, &f.palette);
            let (width, height, rows, cols) = (f.width, f.height, &f.rows, &f.cols);
            let layout = self.layout();
            snapshot.append_color(&palette.header, &rect(0.0, 0.0, width, geom.header_h));
            snapshot.append_color(&palette.header, &rect(0.0, 0.0, geom.header_w, height));

            let (start, end) = f.selection.rect();
            let wash = with_alpha(palette.accent, 0.30);

            snapshot.push_clip(&rect(geom.header_w, 0.0, width, geom.header_h));
            for col in cols.clone() {
                let cell = geom.cell_rect(0, col);
                let head = Rect {
                    y: 0.0,
                    h: geom.header_h,
                    ..cell
                };
                // Which column am I in — the question a header answers, and the reason the
                // selection reaches up here at all.
                if (start.col..=end.col).contains(&col) {
                    snapshot.append_color(&wash, &rect(head.x, head.y, head.w, head.h));
                }
                layout.set_text(&sheet_core::formula::lex::column_name(col));
                let (w, h) = layout.pixel_size();
                draw_text(snapshot, &layout, palette.header_text, &head, head, w, h, Align::Center);
            }
            snapshot.pop();

            snapshot.push_clip(&rect(0.0, geom.header_h, geom.header_w, height));
            for row in rows.clone() {
                let cell = geom.cell_rect(row, 0);
                let head = Rect {
                    x: 0.0,
                    w: geom.header_w,
                    ..cell
                };
                if (start.row..=end.row).contains(&row) {
                    snapshot.append_color(&wash, &rect(head.x, head.y, head.w, head.h));
                }
                layout.set_text(&(row + 1).to_string());
                let (w, h) = layout.pixel_size();
                draw_text(snapshot, &layout, palette.header_text, &head, head, w, h, Align::Center);
            }
            snapshot.pop();

            let line = with_alpha(palette.lines, 1.0);
            snapshot.append_color(&line, &rect(0.0, geom.header_h - 1.0, width, 1.0));
            snapshot.append_color(&line, &rect(geom.header_w - 1.0, 0.0, 1.0, height));
        }
    }

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Align {
        Left,
        Center,
        Right,
    }

    /// What a value's type says about where it sits in its cell.
    ///
    /// The spreadsheet convention, and it carries information: a number that reads as text
    /// is visibly left-aligned, which is how a user spots the import that went wrong.
    /// Errors are centred, as LibreOffice and Excel both draw them.
    fn alignment(value: &CellValue) -> Align {
        match value {
            CellValue::Number(_) => Align::Right,
            CellValue::Bool(_) => Align::Center,
            CellValue::Text(s) if FormulaError::from_name(s).is_some() => Align::Center,
            _ => Align::Left,
        }
    }

    /// Draw `layout` aligned within `cell`, clipped to `paint` — which is the same
    /// rectangle unless the text is overflowing into its neighbours.
    #[allow(clippy::too_many_arguments)]
    fn draw_text(
        snapshot: &gtk::Snapshot,
        layout: &pango::Layout,
        color: gtk::gdk::RGBA,
        cell: &Rect,
        paint: Rect,
        text_w: i32,
        text_h: i32,
        align: Align,
    ) {
        let text_w = f64::from(text_w);
        let x = match align {
            Align::Left => cell.x + PAD,
            Align::Center => cell.x + (cell.w - text_w) / 2.0,
            Align::Right => cell.x + cell.w - PAD - text_w,
        };
        let y = cell.y + (cell.h - f64::from(text_h)) / 2.0;
        snapshot.push_clip(&rect(paint.x, paint.y, paint.w, paint.h));
        snapshot.save();
        snapshot.translate(&graphene::Point::new(x as f32, y as f32));
        snapshot.append_layout(layout, &color);
        snapshot.restore();
        snapshot.pop();
    }

    /// A byte offset as `GtkEditable` counts positions: in characters.
    fn caret_at(text: &str, byte: usize) -> i32 {
        text.get(..byte).map_or(-1, |head| head.chars().count() as i32)
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
            // Shift+Tab arrives as its own keyval, not as Tab with a modifier.
            K::Tab | K::KP_Tab | K::ISO_Left_Tab => Key::Tab,
            K::Return | K::KP_Enter => Key::Return,
            K::Escape => Key::Escape,
            K::Delete | K::KP_Delete => Key::Delete,
            K::BackSpace => Key::Backspace,
            K::F2 => Key::F2,
            other => other.to_unicode().map_or(Key::Other, Key::Char),
        }
    }

    fn rect(x: f64, y: f64, w: f64, h: f64) -> graphene::Rect {
        graphene::Rect::new(x as f32, y as f32, w as f32, h as f32)
    }

    /// One `configure` call per adjustment, so a resize is one notification rather than
    /// six.
    fn configure(
        adjustment: Option<&gtk::Adjustment>,
        page: f64,
        used: f64,
        step: f64,
        limit: f64,
    ) {
        let Some(adjustment) = adjustment else { return };
        let upper = (used + page).max(adjustment.value() + page).min(limit);
        let value = adjustment.value().clamp(0.0, (upper - page).max(0.0));
        adjustment.configure(
            value,
            0.0,
            upper,
            // A wheel notch is three rows, which is what every other grid does.
            step * 3.0,
            (page - step).max(step),
            page,
        );
    }
}
