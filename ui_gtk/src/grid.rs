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

use std::cell::{Cell, OnceCell, RefCell};
use std::sync::Arc;

use libadwaita::gtk;
use libadwaita::prelude::*;

use gtk::glib;
use libadwaita::subclass::prelude::ObjectSubclassIsExt;
use sheet_core::{App, Pos, a1};

use crate::geom::{GridGeom, MAX_COLS, MAX_ROWS, Sizes};
use crate::keymap::Selection;

/// Pixels to an ODF millimetre.
///
/// A length in a document is physical (§5.4) and a widget is not, so something has to
/// choose. 96 dpi is the toolkit's own notional density — GTK scales the whole widget for a
/// HiDPI display on top of this, so a column set to 2.5cm is 2.5cm on a correctly configured
/// screen and consistent everywhere else.
const PX_PER_MM: f64 = 96.0 / 25.4;

/// The narrowest a drag may make a track. Zero is legal in ODF and means *hidden*, which is
/// a feature with its own UI rather than something to arrive at by dragging past the edge.
const MIN_TRACK: f64 = 6.0;

/// How far the view may be zoomed either way. Past these a cell is either unreadable or a
/// single cell fills the window, and neither is a view of a spreadsheet.
const ZOOM_RANGE: std::ops::RangeInclusive<f64> = 0.25..=4.0;

/// One wheel notch or one press of the zoom keys, as a factor rather than a step: zooming
/// out and back in again lands where it started.
pub const ZOOM_STEP: f64 = 1.1;

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
        self.invalidate();
    }

    /// The document changed: repaint, and forget what was measured from it.
    ///
    /// Every write goes through `App` and every write notifies the observer, so this one
    /// call from the observer's end covers the whole shell — including this widget's own
    /// edits, which reach the core the same way.
    pub fn invalidate(&self) {
        self.imp().auto_rows.replace(None);
        self.queue_draw();
    }

    pub fn zoom(&self) -> f64 {
        self.imp().zoom.get()
    }

    /// Scale the view. Clamped to [`ZOOM_RANGE`]; a factor of 1 is the document at the
    /// toolkit's own idea of its size.
    pub fn set_zoom(&self, zoom: f64) {
        let zoom = zoom.clamp(*ZOOM_RANGE.start(), *ZOOM_RANGE.end());
        if zoom == self.zoom() {
            return;
        }
        self.imp().zoom.set(zoom);
        // Row heights are measured unzoomed, so they survive; the scrollbars and the editor
        // are both sized in pixels and do not.
        if self.imp().mode.get().is_editing() {
            self.imp().restyle_formula();
        }
        self.queue_resize();
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

    /// Where the caret is in the shared buffer, as a byte offset — what the signature hint
    /// and the completion both key off.
    pub fn caret(&self) -> usize {
        self.imp().caret()
    }

    pub fn is_editing(&self) -> bool {
        self.imp().mode.get().is_editing()
    }

    /// Mirror the formula bar's caret into the in-cell editor's own.
    ///
    /// The two are separate `GtkEditable`s over one shared buffer, so each keeps its own
    /// cursor position — but [`Self::caret`] only ever reads the in-cell editor's, which
    /// point mode (`pointing()`, in this file) decides off. Without this, typing `=SUM(` in
    /// the formula bar leaves the in-cell editor's position wherever it last was (0, most of
    /// the time), so a click afterwards judges point mode against the wrong spot in the text
    /// and just commits the edit instead — the formula bar calls this on every caret move so
    /// the two never disagree about where "the" caret is.
    pub fn set_caret(&self, position: i32) {
        self.imp().editor.set_position(position);
    }

    /// Throw the edit away and put the cell back the way it was.
    pub fn cancel_edit(&self) {
        self.imp().cancel();
    }

    /// Told whenever the caret moves without the text changing — an arrow key through a
    /// formula, or this widget placing it after a reference it just wrote. The signature
    /// hint depends on *where* the caret is, so a text-changed signal alone would leave it
    /// one keystroke behind.
    pub fn connect_caret_moved(&self, f: impl Fn() + 'static) {
        self.imp().on_caret.borrow_mut().push(Box::new(f));
    }

    /// Told whenever an edit opens or closes — the formula bar's ✓/✗ buttons, and anything
    /// else that only makes sense mid-edit.
    pub fn connect_editing_changed(&self, f: impl Fn(bool) + 'static) {
        self.imp().on_editing.borrow_mut().push(Box::new(f));
    }

    /// Told when something happened that a user has to hear about. Chrome turns these into
    /// a toast or a banner; the grid does not know which.
    pub fn connect_notice(&self, f: impl Fn(Notice) + 'static) {
        self.imp().on_notice.borrow_mut().push(Box::new(f));
    }

    /// Tell the chrome something, from a part of the chrome that has no toast of its own —
    /// the format strip, whose writes can be refused for the same reasons the grid's can.
    pub fn report(&self, notice: Notice) {
        self.imp().notice(notice);
    }

    /// The selection as a rectangle the core will accept: clamped to the sheet's used extent,
    /// never smaller than the anchor cell.
    ///
    /// A whole-column selection is 1048576 rows, and every per-cell operation — formatting,
    /// styling, summing — has to stop at the data or ask for a million writes.
    ///
    /// ponytail: which means "format column A" formats the *used* part of column A, where a
    /// spreadsheet would put a default style on the column itself. That needs the column
    /// default styles the reader already honours on the way in (see `odf/read.rs`) to become
    /// something the model can *write*, which is M8's neighbourhood.
    pub fn target(&self) -> Option<(usize, Pos, Pos)> {
        let app = self.imp().app.borrow().clone()?;
        let sheet = self.sheet();
        let (start, end) = self.selection().rect();
        let (rows, cols) = app.used_extent(sheet).ok()?;
        Some((
            sheet,
            start,
            Pos::new(
                end.row.min(rows.saturating_sub(1)).max(start.row),
                end.col.min(cols.saturating_sub(1)).max(start.col),
            ),
        ))
    }

    /// Called after every selection change, with the selection that resulted — what the
    /// status bar and the formula bar are driven from. The grid does not know what they
    /// are; it reports, and chrome decides.
    pub fn connect_selection_changed(&self, f: impl Fn(Selection) + 'static) {
        self.imp().on_selection.borrow_mut().push(Box::new(f));
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
    /// The core refused, and said why — a rectangle too large to format, a sheet that is
    /// gone. Carries the core's own message rather than a rephrasing of it.
    Refused(String),
}

impl Default for Grid {
    fn default() -> Self {
        glib::Object::builder().build()
    }
}

/// What clicking on something selects, as arithmetic — free of the widget so it can be
/// tested without a display. See `Grid::selection_for` for why the ends are this way round.
fn selection_for_hit(hit: crate::geom::Hit) -> Selection {
    use crate::geom::Hit;
    match hit {
        Hit::Cell { row, col } => Selection::at(Pos::new(row, col)),
        Hit::ColHeader(col) | Hit::ColEdge(col) => Selection {
            anchor: Pos::new(MAX_ROWS - 1, col),
            active: Pos::new(0, col),
        },
        Hit::RowHeader(row) | Hit::RowEdge(row) => Selection {
            anchor: Pos::new(row, MAX_COLS - 1),
            active: Pos::new(row, 0),
        },
        Hit::Corner => Selection {
            anchor: Pos::new(MAX_ROWS - 1, MAX_COLS - 1),
            active: Pos::new(0, 0),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::Hit;

    /// The whole point of the anchor/active order: the cell the view scrolls to stays next
    /// to the header that was clicked, while the rectangle still covers the whole track.
    #[test]
    fn a_header_selects_its_whole_track_without_scrolling_off_the_sheet() {
        let row = selection_for_hit(Hit::RowHeader(4));
        assert_eq!(row.active, Pos::new(4, 0));
        assert_eq!(row.rect(), (Pos::new(4, 0), Pos::new(4, MAX_COLS - 1)));

        let col = selection_for_hit(Hit::ColHeader(2));
        assert_eq!(col.active, Pos::new(0, 2));
        assert_eq!(col.rect(), (Pos::new(0, 2), Pos::new(MAX_ROWS - 1, 2)));

        let all = selection_for_hit(Hit::Corner);
        assert_eq!(all.active, Pos::new(0, 0));
        assert_eq!(all.rect(), (Pos::new(0, 0), Pos::new(MAX_ROWS - 1, MAX_COLS - 1)));
    }

    /// Dragging down the row header keeps the far anchor, so the rectangle grows over rows
    /// and still spans every column.
    #[test]
    fn dragging_from_a_row_header_grows_the_row_range() {
        let start = selection_for_hit(Hit::RowHeader(4));
        let dragged = Selection {
            anchor: start.anchor,
            active: selection_for_hit(Hit::RowHeader(7)).active,
        };
        assert_eq!(dragged.rect(), (Pos::new(4, 0), Pos::new(7, MAX_COLS - 1)));
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
    use sheet_core::style;

    use crate::geom::{Hit, Rect};
    use crate::keymap::{self, Action, Dir, Extent, Key, Mods};
    use crate::state::{self, Mode, Outcome, Seed};
    use crate::theme::{Palette, with_alpha};

    use super::Notice;

    // What the grid tells chrome. Lists rather than slots: the formula bar wants the
    // editing signal for its buttons *and* for its hint, and a `connect_` that quietly
    // replaced the previous subscriber would leave one of them never called.
    type SelectionHook = Box<dyn Fn(Selection)>;
    type NoticeHook = Box<dyn Fn(Notice)>;
    type EditingHook = Box<dyn Fn(bool)>;
    type CaretHook = Box<dyn Fn()>;

    /// Space either side of a cell's text.
    const PAD: f64 = 4.0;
    /// Space above and below it, which is what makes the default row taller than a line.
    const ROW_PAD: f64 = 8.0;
    /// How much sheet is measured for natural row heights. A row above the view still
    /// displaces the ones below it, so this pass cannot be limited to what is on screen —
    /// past this much document every row keeps the default height instead.
    const AUTO_HEIGHT_CELLS: u64 = 200_000;
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
        /// The cells this paint draws, read **once** — three passes need them (backgrounds
        /// under the selection wash, borders over the grid lines, then the text), and asking
        /// three times would take the document's lock three times a frame.
        ///
        /// Wider than `cols` by [`OVERFLOW_MARGIN`] either side, so a label anchored just
        /// off-screen still reaches into the view and "is the neighbour empty" needs no
        /// second read. `None` when there is no document.
        cells: Option<sheet_core::Viewport>,
    }

    pub struct Grid {
        pub app: RefCell<Option<Arc<App>>>,
        pub sheet: Cell<usize>,
        pub hadjustment: RefCell<Option<gtk::Adjustment>>,
        pub vadjustment: RefCell<Option<gtk::Adjustment>>,
        pub hscroll_policy: Cell<gtk::ScrollablePolicy>,
        pub vscroll_policy: Cell<gtk::ScrollablePolicy>,
        pub metrics: Cell<Metrics>,
        /// The view's scale, applied to every pixel this widget derives — the metrics, the
        /// document's own lengths, and the font, which takes it as a Pango *scale* so that a
        /// cell with a size of its own zooms too.
        ///
        /// Nothing measured is stored zoomed: a natural row height is a document-space
        /// number like the heights it sits beside, and only [`Grid::geom`] multiplies.
        pub zoom: Cell<f64>,
        /// Natural heights for rows the document did not size, measured from what is in
        /// them — `None` until the next paint measures again.
        ///
        /// ponytail: the whole used extent is re-measured after any change, where measuring
        /// only the rows a change touched would do. Bounded instead by [`AUTO_HEIGHT_CELLS`],
        /// so the cost has a ceiling rather than a cache to invalidate correctly.
        pub auto_rows: RefCell<Option<Vec<(u32, f64)>>>,
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
        /// A track being resized, in pixels — presentation state until the pointer is
        /// released, at which point it becomes one core write and one undo entry.
        pub resize: Cell<Option<Resize>>,
        pub on_selection: RefCell<Vec<SelectionHook>>,
        pub on_notice: RefCell<Vec<NoticeHook>>,
        pub on_editing: RefCell<Vec<EditingHook>>,
        pub on_caret: RefCell<Vec<CaretHook>>,
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
        /// The reference being pointed at, if any: the cells, and the byte range of the
        /// text this widget last wrote for them.
        pub pending: RefCell<Option<Pending>>,
        /// Set while the buffer is being rewritten from here, so that the change signal can
        /// tell "the user typed" from "we wrote a reference".
        pub applying: Cell<bool>,
        /// Tab-column memory: the column a run of Tabs started in, so that Enter goes back
        /// to it. One integer, and disproportionately loved.
        pub tab_origin: Cell<Option<u32>>,
        /// The autocomplete popover, parented to this widget so it appears under the cell
        /// being typed into rather than at the formula bar.
        pub completion: OnceCell<crate::formula_ux::Completion>,
    }

    /// A column or row being dragged wider.
    #[derive(Clone, Copy, Debug)]
    pub struct Resize {
        /// [`Hit::ColEdge`] or [`Hit::RowEdge`] — which boundary was grabbed.
        pub track: Hit,
        pub size: f64,
    }

    /// A reference being pointed at.
    #[derive(Clone, Debug)]
    pub struct Pending {
        /// Byte range in the buffer holding the text this reference was rendered to.
        pub span: std::ops::Range<usize>,
        pub anchor: Pos,
        pub active: Pos,
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
                zoom: Cell::new(1.0),
                auto_rows: RefCell::new(None),
                palette: RefCell::new(None),
                layout: RefCell::new(None),
                selection: Cell::new(Selection::default()),
                drag: Cell::new(None),
                resize: Cell::new(None),
                on_selection: RefCell::new(Vec::new()),
                on_notice: RefCell::new(Vec::new()),
                on_editing: RefCell::new(Vec::new()),
                on_caret: RefCell::new(Vec::new()),
                mode: Cell::new(Mode::default()),
                editor: gtk::Text::new(),
                buffer: gtk::EntryBuffer::default(),
                editor_rect: Cell::new(Rect {
                    x: 0.0,
                    y: 0.0,
                    w: 0.0,
                    h: 0.0,
                }),
                pending: RefCell::new(None),
                applying: Cell::new(false),
                tab_origin: Cell::new(None),
                completion: OnceCell::new(),
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

        /// A widget with children has to unparent them, or GTK complains at teardown — and
        /// a popover is a child too, even though it draws in its own surface.
        fn dispose(&self) {
            self.editor.unparent();
            if let Some(completion) = self.completion.get() {
                completion.dispose();
            }
        }

        fn constructed(&self) {
            self.parent_constructed();
            let widget = self.obj();
            widget.set_focusable(true);

            // The editor is a child of the grid rather than an overlay: an overlay
            // positions in widget coordinates and would re-derive the scroll arithmetic
            // every frame, where a child is allocated by the same `cell_rect` that draws.
            let _ = self
                .completion
                .set(crate::formula_ux::Completion::new(&*widget));
            self.editor.set_buffer(&self.buffer);
            // The caret moving is its own event: the completion and the signature hint are
            // both questions about where it is, not about what the text says.
            self.editor.connect_cursor_position_notify(glib::clone!(
                #[weak(rename_to = grid)]
                widget,
                move |_| grid.imp().caret_moved()
            ));
            self.editor.set_parent(&*widget);
            self.editor.set_visible(false);
            self.editor.add_css_class("sheet-editor");

            // Anything the *user* types finalises the reference being pointed at: the next
            // arrow key then starts a new one rather than moving the last.
            self.buffer.connect_text_notify(glib::clone!(
                #[weak(rename_to = grid)]
                widget,
                move |_| {
                    let imp = grid.imp();
                    if !imp.applying.get() {
                        imp.pending.replace(None);
                    }
                    imp.restyle_formula();
                    imp.update_completion();
                    // The editor grows with what is typed into it.
                    grid.queue_allocate();
                    grid.queue_draw();
                }
            ));

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
                move |_, presses, x, y| {
                    if presses != 2 {
                        return;
                    }
                    // On a boundary the second click is a fit, not an edit — the boundary is
                    // in the header band, where there is nothing to type into anyway.
                    match grid.imp().geom().hit(x, y) {
                        Hit::ColEdge(col) => grid.imp().autofit(col),
                        Hit::RowEdge(row) => grid.imp().clear_height(row),
                        _ => grid.imp().begin(Seed::Cell, true),
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
                move |_, _, _| {
                    grid.imp().drag.set(None);
                    grid.imp().commit_resize();
                }
            ));
            widget.add_controller(drag);

            // The pointer says what a press would do before it happens, which is the only
            // thing that makes a 4px target discoverable.
            let motion = gtk::EventControllerMotion::new();
            motion.connect_motion(glib::clone!(
                #[weak(rename_to = grid)]
                widget,
                move |_, x, y| {
                    let cursor = match grid.imp().geom().hit(x, y) {
                        Hit::ColEdge(_) => Some("col-resize"),
                        Hit::RowEdge(_) => Some("row-resize"),
                        _ => None,
                    };
                    grid.set_cursor_from_name(cursor);
                }
            ));
            widget.add_controller(motion);

            // Ctrl+wheel zooms; every other wheel event travels on to the scrolled window,
            // which is the one that knows how to scroll.
            let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
            scroll.connect_scroll(glib::clone!(
                #[weak(rename_to = grid)]
                widget,
                #[upgrade_or]
                glib::Propagation::Proceed,
                move |controller, _, dy| {
                    if !controller
                        .current_event_state()
                        .contains(gtk::gdk::ModifierType::CONTROL_MASK)
                    {
                        return glib::Propagation::Proceed;
                    }
                    grid.set_zoom(grid.zoom() * ZOOM_STEP.powf(-dy));
                    glib::Propagation::Stop
                }
            ));
            widget.add_controller(scroll);
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
            // Measured from the text, not from the widget: a `gtk::Text` asks for a width
            // in characters and knows nothing about what it is holding, so an unmeasured
            // editor clips a formula at the column's edge.
            let layout = self.layout();
            // Measured in what the editor actually draws with — the cell's font and the zoom
            // — or the editor is sized for text a different size to the text in it.
            layout.set_attributes(self.editor.attributes().as_ref());
            layout.set_width(-1);
            layout.set_text(&self.buffer.text());
            let wanted = f64::from(layout.pixel_size().0) + 4.0 * PAD;
            let w = wanted.max(cell.w).min((width - cell.x).max(cell.w));
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
            let rows = geom.visible_rows(height);
            let cols = geom.visible_cols(width);
            let frame = Frame {
                snapshot,
                palette: self.palette(),
                cells: self.read(&rows, &cols),
                rows,
                cols,
                geom,
                width,
                height,
                selection: self.selection.get(),
            };

            snapshot.append_color(&frame.palette.background, &rect(0.0, 0.0, width, height));

            snapshot.push_clip(&rect(
                frame.geom.header_w,
                frame.geom.header_h,
                width - frame.geom.header_w,
                height - frame.geom.header_h,
            ));
            // Cell backgrounds go **under** the selection wash: a tint over a yellow cell
            // still reads as selected, where a yellow cell over the tint hides it.
            self.draw_backgrounds(&frame);
            self.draw_selection(&frame);
            self.draw_lines(&frame);
            // And a cell's own borders over the grid lines, which is what makes a ruled
            // table look ruled rather than slightly darker.
            self.draw_borders(&frame);
            self.draw_cells(&frame);
            self.draw_active(&frame);
            self.draw_references(&frame);
            snapshot.pop();

            self.draw_headers(&frame);
            // Last, and only while editing: a real child widget, drawn over its cell.
            if self.editor.is_visible() {
                widget.snapshot_child(&self.editor, snapshot);
            }
        }
    }

    impl Grid {
        /// What the document says about one axis, in pixels.
        ///
        /// ponytail: read afresh every time rather than cached and invalidated. A document
        /// sizes a handful of tracks out of sixteen thousand, so this is a short walk over a
        /// `BTreeMap` and cannot go stale; cache it against a document-change token if a
        /// profiler ever blames it.
        fn track_lengths(&self, rows: bool) -> Vec<(u32, f64)> {
            let sheet = self.sheet.get();
            let app = self.app.borrow();
            let sizes = app
                .as_ref()
                .and_then(|app| match rows {
                    true => app.row_heights(sheet).ok(),
                    false => app.col_widths(sheet).ok(),
                })
                .unwrap_or_default();
            sizes
                .iter()
                .filter_map(|(i, len)| Some((*i, style::length_mm(len)? * PX_PER_MM)))
                .collect()
        }

        /// The columns, unzoomed — what a natural height is measured against.
        fn col_sizes(&self) -> Sizes {
            Sizes::new(self.metrics.get().col_width, MAX_COLS, self.track_lengths(false))
        }

        fn geom(&self) -> GridGeom {
            let m = self.metrics.get();
            let zoom = self.zoom.get();
            // The document's own heights are given *after* the measured ones, because
            // `Sizes::new` keeps the last entry for an index: a row the document sized keeps
            // that size and clips, which is what an explicit height means.
            let mut heights = self.auto_heights();
            heights.extend(self.track_lengths(true));
            let mut rows = Sizes::new(m.row_height, MAX_ROWS, heights).scaled(zoom);
            let mut cols = self.col_sizes().scaled(zoom);
            // A resize in progress is painted before it is stored, so the whole grid reflows
            // under the pointer rather than a guide line standing in for it. It arrives in
            // screen pixels, which is why it goes on after the zoom rather than before.
            match self.resize.get() {
                Some(Resize { track: Hit::ColEdge(col), size }) => cols = cols.with(col, size),
                Some(Resize { track: Hit::RowEdge(row), size }) => rows = rows.with(row, size),
                _ => {}
            }
            GridGeom {
                header_w: m.header_w * zoom,
                header_h: m.header_h * zoom,
                rows,
                cols,
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

        /// Natural heights for the rows that need one, measured once per document change.
        ///
        /// A row is only ever *grown*: the default height already fits a line of the widget's
        /// font, so what can overflow it is a cell that wraps onto a second line or one whose
        /// style asks for a bigger font. Both need a style, and a cell without one is skipped
        /// without laying anything out — which is what keeps this a cheap pass over a sheet
        /// where nine cells in ten are plain.
        fn auto_heights(&self) -> Vec<(u32, f64)> {
            if let Some(measured) = self.auto_rows.borrow().as_ref() {
                return measured.clone();
            }
            let measured = self.measure_rows();
            self.auto_rows.replace(Some(measured.clone()));
            measured
        }

        fn measure_rows(&self) -> Vec<(u32, f64)> {
            let Some(app) = self.app.borrow().clone() else {
                return Vec::new();
            };
            let sheet = self.sheet.get();
            let Ok((used_rows, used_cols)) = app.used_extent(sheet) else {
                return Vec::new();
            };
            if used_rows == 0 || used_cols == 0 || u64::from(used_rows) * u64::from(used_cols) > AUTO_HEIGHT_CELLS {
                return Vec::new();
            }
            let Ok(viewport) = app.get_viewport(sheet, 0..used_rows, 0..used_cols) else {
                return Vec::new();
            };

            let default = self.metrics.get().row_height;
            let cols = self.col_sizes();
            let layout = self.layout();
            let mut measured = Vec::new();
            for row in 0..used_rows {
                let mut tallest: f64 = 0.0;
                for col in 0..used_cols {
                    let Some(style) = viewport.style(row, col) else {
                        continue;
                    };
                    let wrapping = style.wrap.as_deref() == Some("wrap");
                    if !wrapping && style.font_size.is_none() {
                        continue;
                    }
                    let Some(text) = viewport.text(row, col).filter(|t| !t.is_empty()) else {
                        continue;
                    };
                    layout.set_attributes(font(style).as_ref());
                    layout.set_width(match wrapping {
                        true => ((cols.size_of(col) - 2.0 * PAD).max(1.0) * f64::from(pango::SCALE)) as i32,
                        false => -1,
                    });
                    layout.set_text(text);
                    tallest = tallest.max(f64::from(layout.pixel_size().1));
                }
                if tallest + ROW_PAD > default {
                    measured.push((row, (tallest + ROW_PAD).ceil()));
                }
            }
            // The layout is shared, so what was set for a measurement has to be unset.
            layout.set_attributes(None);
            layout.set_width(-1);
            measured
        }

        /// The one read a paint makes — [`Frame::cells`].
        fn read(
            &self,
            rows: &std::ops::Range<u32>,
            cols: &std::ops::Range<u32>,
        ) -> Option<sheet_core::Viewport> {
            let app = self.app.borrow();
            let app = app.as_ref()?;
            let fetch = cols.start.saturating_sub(OVERFLOW_MARGIN)
                ..(cols.end.saturating_add(OVERFLOW_MARGIN)).min(MAX_COLS);
            app.get_viewport(self.sheet.get(), rows.clone(), fetch).ok()
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

        /// What a layout draws with: the cell's own attributes, plus the zoom as a font
        /// *scale* — a multiplier over whatever size applies, so a cell that set its own size
        /// zooms with everything else.
        fn attrs(&self, cell: Option<pango::AttrList>) -> Option<pango::AttrList> {
            let zoom = self.zoom.get();
            if zoom == 1.0 {
                return cell;
            }
            let attrs = cell.unwrap_or_default();
            attrs.insert(pango::AttrFloat::new_scale(zoom));
            Some(attrs)
        }

        /// Drop everything derived from the style and derive it again.
        fn restyle(&self) {
            self.palette.replace(None);
            self.layout.replace(None);
            // Row heights are measured in the widget's font, so a new one remeasures them.
            self.auto_rows.replace(None);
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
            let row_height = (line + ROW_PAD).ceil();
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
                geom.cols.offset_of(used_cols.min(MAX_COLS)),
                geom.cols.size_of(0),
                geom.cols.total(),
            );
            configure(
                self.vadjustment.borrow().as_ref(),
                page_h,
                geom.rows.offset_of(used_rows.min(MAX_ROWS)),
                geom.rows.size_of(0),
                geom.rows.total(),
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
            // While the list is up it owns the keys that pick from it, and nothing else —
            // typing keeps narrowing it, and every other key means what it always means.
            if let Some(completion) = self.completion.get().filter(|c| c.is_visible()) {
                match key_of(keyval) {
                    Key::Up => {
                        completion.step(-1);
                        return glib::Propagation::Stop;
                    }
                    Key::Down => {
                        completion.step(1);
                        return glib::Propagation::Stop;
                    }
                    Key::Tab | Key::Return => {
                        if let Some((span, insert)) = completion.accept() {
                            self.replace(span, &insert);
                            return glib::Propagation::Stop;
                        }
                    }
                    Key::Escape => {
                        completion.hide();
                        return glib::Propagation::Stop;
                    }
                    _ => {}
                }
            }

            let text = self.buffer.text().to_string();
            let at = state::Where {
                mode: self.mode.get(),
                text: &text,
                caret: self.caret(),
                pending: self.pending.borrow().is_some(),
            };
            let action = match state::on_key(at, key_of(keyval), mods) {
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
                Outcome::Point { motion, extend } => {
                    self.point(motion, extend);
                    return glib::Propagation::Stop;
                }
                Outcome::CycleAbsolute => {
                    self.cycle_absolute();
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
            self.pending.replace(None);
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
            self.restyle_formula();
            self.update_completion();
            self.obj().queue_allocate();
            self.obj().queue_draw();
        }

        // --- pointing (doc/gtk-shell.md's formula UX) ---

        /// Where the caret is, as a byte offset into the buffer.
        ///
        /// The in-cell editor's, whenever an edit is open — it is the widget being typed
        /// into, and after a reference is written this widget puts the caret there itself.
        /// With no edit open there is no caret, and the end of the text is the one answer
        /// that is never nonsense. (The formula bar's own caret is the formula bar's to
        /// report; it asks this only when it does not have the focus.)
        pub fn caret(&self) -> usize {
            let text = self.buffer.text().to_string();
            if !self.editor.is_visible() {
                return text.len();
            }
            let position = self.editor.position().max(0) as usize;
            text.char_indices()
                .nth(position)
                .map_or(text.len(), |(byte, _)| byte)
        }

        /// Move — or start — the reference being pointed at.
        fn point(&self, motion: keymap::Motion, extend: bool) {
            let pending = self.pending.borrow().clone();
            let from = match &pending {
                Some(pending) => Selection {
                    anchor: pending.anchor,
                    active: pending.active,
                },
                // The first arrow points one cell away from the one being edited, which is
                // where the eye already is.
                None => Selection::at(self.selection.get().active),
            };
            let app = self.app.borrow().clone();
            let sheet = self.sheet.get();
            let occupied = |pos: Pos| {
                app.as_ref()
                    .and_then(|app| app.get(sheet, pos).ok())
                    .is_some_and(|value| !value.is_empty())
            };
            let moved = keymap::moved(from, motion, extend, self.extent(), &occupied);
            self.set_pending(moved, pending.map(|p| p.span));
        }

        /// Write the reference `selection` names into the buffer, replacing whatever was
        /// written for it last time.
        fn set_pending(&self, selection: Selection, span: Option<std::ops::Range<usize>>) {
            let (start, end) = selection.rect();
            let text = display::reference_text(&sheet_core::a1::reference(None, start, end));
            let span = span.unwrap_or_else(|| {
                let caret = self.caret();
                caret..caret
            });
            // A click here (`GestureDrag::connect_drag_begin`) grabs focus onto the grid
            // widget itself, not the editor, so typing afterwards would have nowhere to land
            // — reclaim it, since every point-mode update (click, drag, arrow key) funnels
            // through here. *Before* the replacement, not after: focusing a `gtk::Text`
            // selects all of its content (the same trap `begin` documents), and the next
            // keystroke would replace the whole formula rather than extend it. `replace`
            // ends by placing the caret, which collapses that selection again.
            self.editor.grab_focus();
            let placed = self.replace(span.clone(), &text);
            self.pending.replace(Some(Pending {
                span: placed,
                anchor: selection.anchor,
                active: selection.active,
            }));
            // The cells being pointed at have to be on screen, or pointing at a cell below
            // the fold means typing blind.
            self.scroll_into_view(selection.active);
            self.obj().queue_draw();
        }

        /// Replace a byte range of the buffer, leaving the caret after what was written.
        /// Returns the range the new text occupies.
        fn replace(&self, span: std::ops::Range<usize>, with: &str) -> std::ops::Range<usize> {
            let full = self.buffer.text().to_string();
            let (head, tail) = (&full[..span.start], &full[span.end.min(full.len())..]);
            let text = format!("{head}{with}{tail}");
            let placed = span.start..span.start + with.len();
            self.applying.set(true);
            self.buffer.set_text(&text);
            self.editor.set_position(caret_at(&text, placed.end));
            self.applying.set(false);
            self.restyle_formula();
            // The buffer's own change signal fired *inside* `set_text`, before the caret
            // moved — so anything that reads the caret has to be told again, now.
            self.caret_moved();
            placed
        }

        /// F4: `B2` → `$B$2` → `B$2` → `$B2` → `B2`, on the reference being pointed at or
        /// the one under the caret.
        fn cycle_absolute(&self) {
            let text = self.buffer.text().to_string();
            let at = self
                .pending
                .borrow()
                .as_ref()
                .map_or_else(|| self.caret(), |pending| pending.span.end);
            let Some((span, replacement)) = state::cycle_absolute(&text, at) else {
                return;
            };
            let placed = self.replace(span, &replacement);
            // The pending reference keeps pointing at the same cells; only its spelling and
            // therefore its length changed.
            if let Some(pending) = self.pending.borrow_mut().as_mut() {
                pending.span = placed;
            }
            self.obj().queue_draw();
        }

        /// Whether a click on the grid should point at cells rather than move the cursor.
        fn pointing(&self) -> bool {
            self.mode.get().is_editing()
                && (self.pending.borrow().is_some()
                    || state::ref_eligible(&self.buffer.text(), self.caret()))
        }

        /// Offer what could be typed next, under the cell being typed into.
        fn update_completion(&self) {
            let Some(completion) = self.completion.get() else {
                return;
            };
            if !self.mode.get().is_editing() {
                return completion.hide();
            }
            let names: Vec<String> = self
                .app
                .borrow()
                .as_ref()
                .map(|app| app.names().into_iter().map(|(name, _)| name).collect())
                .unwrap_or_default();
            completion.update(
                &self.buffer.text(),
                self.caret(),
                &names,
                self.editor_rect.get(),
            );
        }

        /// Fan out a caret move: what can be offered and what the hint says both change.
        fn caret_moved(&self) {
            self.update_completion();
            for hook in self.on_caret.borrow().iter() {
                hook();
            }
        }

        /// Colour the references in the editor. The formula bar does the same to its own
        /// copy from the same function, so the two cannot disagree.
        /// What the in-cell editor draws with: the reference colouring, over the cell's own
        /// font and the zoom.
        ///
        /// The editor is a real `gtk::Text` child rather than something this widget draws, so
        /// what a cell looks like has to be told to it rather than falling out of the same
        /// layout — Pango attributes rather than CSS, because they are what the grid uses for
        /// the same cell and what the colouring already speaks.
        ///
        /// The font only: weight, slant, size and the zoom. The *colour* stays the theme's,
        /// because the reference colouring owns the foreground here and a cell colour
        /// underneath it would be a second opinion about the same bytes.
        pub fn restyle_formula(&self) {
            let text = self.buffer.text().to_string();
            let dark = crate::theme::is_dark(&self.palette());
            let attrs = crate::theme::reference_attributes(&text, dark);
            let pos = self.selection.get().active;
            let style = self
                .app
                .borrow()
                .as_ref()
                .and_then(|app| app.style_at(self.sheet.get(), pos).ok().flatten());
            if let Some(cell) = self.attrs(style.as_ref().and_then(font)) {
                for attribute in cell.attributes() {
                    attrs.insert(attribute);
                }
            }
            self.editor.set_attributes(Some(&attrs));
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
            let (to, tab_origin) = state::after_commit(from, direction, self.tab_origin.get());
            self.tab_origin.set(tab_origin);
            self.set_selection(Selection::at(Pos::new(
                to.row.min(MAX_ROWS - 1),
                to.col.min(MAX_COLS - 1),
            )));
        }

        /// Throw the edit away. The document is never touched, so there is nothing to undo.
        pub fn cancel(&self) {
            self.end_edit();
            self.refresh_buffer();
            self.obj().grab_focus();
            self.obj().queue_draw();
        }

        fn end_edit(&self) {
            if let Some(completion) = self.completion.get() {
                completion.hide();
            }
            self.pending.replace(None);
            self.mode.set(Mode::Ready);
            self.editor.set_visible(false);
            self.editing_changed(false);
            self.obj().grab_focus();
        }

        fn editing_changed(&self, editing: bool) {
            for hook in self.on_editing.borrow().iter() {
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

        pub fn notice(&self, notice: Notice) {
            for hook in self.on_notice.borrow().iter() {
                hook(notice.clone());
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

        /// The current size of the track a boundary belongs to, or `None` if this is not a
        /// boundary at all — which is also how `press` tells a resize from a selection.
        fn track_size_of(&self, hit: Hit) -> Option<f64> {
            let geom = self.geom();
            match hit {
                Hit::ColEdge(col) => Some(geom.cols.size_of(col)),
                Hit::RowEdge(row) => Some(geom.rows.size_of(row)),
                _ => None,
            }
        }

        /// The end of a resize drag: the pixels become an ODF length and one undo entry.
        fn commit_resize(&self) {
            let Some(resize) = self.resize.take() else { return };
            let Some(app) = self.app.borrow().clone() else { return };
            let sheet = self.sheet.get();
            // The drag happened on screen, so the zoom comes back out before the pixels
            // become a physical length.
            let length = Some(style::mm_length(resize.size / self.zoom.get() / PX_PER_MM));
            let result = match resize.track {
                Hit::ColEdge(col) => app.set_col_width(sheet, col..col + 1, length),
                Hit::RowEdge(row) => app.set_row_height(sheet, row..row + 1, length),
                _ => return,
            };
            if let Err(error) = result {
                self.notice(Notice::Refused(error.to_string()));
            }
            self.obj().queue_draw();
        }

        /// Double-clicking a column boundary: wide enough for the widest thing in the
        /// column, which is what every spreadsheet does with that gesture.
        ///
        /// The shell measures and the core stores — text width is a font question and the
        /// core has no font. Only the used extent is measured, because a column of a million
        /// empty cells has no widest thing in it.
        ///
        /// A row needs no equivalent: a row without a height of its own is *already* fitted
        /// to what is in it ([`Grid::measure_rows`]), so double-clicking a row boundary
        /// clears the explicit height and the fit is what is left.
        fn autofit(&self, col: u32) {
            let Some(app) = self.app.borrow().clone() else { return };
            let sheet = self.sheet.get();
            let Ok((rows, _)) = app.used_extent(sheet) else { return };
            let layout = self.layout();
            layout.set_width(-1);

            // Measured unzoomed, because what is stored is a length in the document rather
            // than a number of pixels on this screen at this scale.
            let mut width: f64 = 0.0;
            if let Ok(viewport) = app.get_viewport(sheet, 0..rows, col..col + 1) {
                for row in 0..rows {
                    let Some(text) = viewport.text(row, col).filter(|t| !t.is_empty()) else {
                        continue;
                    };
                    layout.set_attributes(viewport.style(row, col).and_then(font).as_ref());
                    layout.set_text(text);
                    width = width.max(f64::from(layout.pixel_size().0));
                }
            }
            layout.set_attributes(None);
            let width = (width + 2.0 * PAD).max(MIN_TRACK);
            let length = Some(style::mm_length(width / PX_PER_MM));
            if let Err(error) = app.set_col_width(sheet, col..col + 1, length) {
                self.notice(Notice::Refused(error.to_string()));
            }
            self.obj().queue_draw();
        }

        /// Double-clicking a row boundary: drop the explicit height and let the row fit
        /// itself again, which `autofit`'s doc comment explains.
        fn clear_height(&self, row: u32) {
            let Some(app) = self.app.borrow().clone() else { return };
            let sheet = self.sheet.get();
            if let Err(error) = app.set_row_height(sheet, row..row + 1, None) {
                self.notice(Notice::Refused(error.to_string()));
            }
            self.obj().queue_draw();
        }

        /// Press: a cell, or a whole column or row from its header.
        fn press(&self, x: f64, y: f64, extend: bool) {
            let hit = self.geom().hit(x, y);
            if self.mode.get().is_editing() {
                // A click *inside* the editor is a caret move, and the event reaches here
                // only because the editor is a child of this widget.
                if self.editor_rect.get().contains(x, y) {
                    return;
                }
                // A click where a reference could go points at cells instead of ending the
                // edit — `=SUM(` then clicking B2 is how most formulas get written.
                if self.pointing()
                    && let Hit::Cell { row, col } = hit
                {
                    self.drag.set(Some(hit));
                    let span = self.pending.borrow().as_ref().map(|p| p.span.clone());
                    self.set_pending(Selection::at(Pos::new(row, col)), span);
                    return;
                }
                // Clicking anywhere else stores the edit, which is what every spreadsheet
                // does and what a user who clicks the next cell means.
                self.commit(None);
            }
            self.drag.set(Some(hit));
            // A press *on* a boundary is a resize rather than a selection — which is why
            // `Hit` distinguishes the two at all.
            if let Some(size) = self.track_size_of(hit) {
                self.resize.set(Some(Resize { track: hit, size }));
                return;
            }
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
            // A resize drag moves the boundary itself. The track's *leading* edge does not
            // move, so measuring against it is stable however far the pointer has gone.
            if let Some(resize) = self.resize.get() {
                let geom = self.geom();
                let size = match resize.track {
                    Hit::ColEdge(col) => x - geom.header_w + geom.scroll_x - geom.cols.offset_of(col),
                    Hit::RowEdge(row) => y - geom.header_h + geom.scroll_y - geom.rows.offset_of(row),
                    _ => return,
                };
                self.resize.set(Some(Resize { size: size.max(MIN_TRACK), ..resize }));
                self.obj().queue_draw();
                return;
            }
            let hit = self.geom().hit(x, y);
            // Dragging while pointing grows the reference rather than the selection, which
            // is what `=SUM(` + drag B2:B4 means. Cloned into its own binding first, not the
            // `if let` scrutinee, or the `Ref` would still be held when `set_pending` below
            // tries to `borrow_mut` the same `RefCell` — a reentrant-borrow panic.
            let pending = self.pending.borrow().clone();
            if let Some(pending) = pending
                && let Hit::Cell { row, col } = hit
            {
                self.set_pending(
                    Selection {
                        anchor: pending.anchor,
                        active: Pos::new(row, col),
                    },
                    Some(pending.span),
                );
                return;
            }
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
        ///
        /// The active cell is the *near* end (row 0 of a column, column A of a row) and the
        /// anchor the far one, not the other way round: [`Self::set_selection`] scrolls the
        /// active cell into view, so an active cell at the sheet's far corner would fling
        /// the view sixteen thousand columns away from the header just clicked. Extending a
        /// drag from the far anchor still covers the same rectangle, since
        /// [`Selection::rect`] normalises.
        fn selection_for(&self, hit: Hit) -> Option<Selection> {
            Some(selection_for_hit(hit))
        }

        /// The one place a selection changes: scroll it into view, repaint, and tell
        /// whoever is listening.
        pub fn set_selection(&self, selection: Selection) {
            self.selection.set(selection);
            self.refresh_buffer();
            self.scroll_into_view(selection.active);
            self.announce_active_cell(selection.active);
            self.obj().queue_draw();
            for hook in self.on_selection.borrow().iter() {
                hook(selection);
            }
        }

        /// The a11y floor (`doc/gtk-shell.md`): a custom-drawn grid has no other way to tell
        /// assistive technology the selection moved, so every move speaks the cell's address
        /// and, if it has one, its display text.
        fn announce_active_cell(&self, pos: Pos) {
            let Some(app) = self.app.borrow().clone() else { return };
            let address = a1::format(None, pos);
            let message = match app.get_viewport(self.sheet.get(), pos.row..pos.row + 1, pos.col..pos.col + 1) {
                Ok(viewport) => match viewport.text(pos.row, pos.col) {
                    Some(text) if !text.is_empty() => format!("{address}: {text}"),
                    _ => address,
                },
                Err(_) => address,
            };
            self.obj()
                .announce(&message, gtk::AccessibleAnnouncementPriority::Medium);
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
            let visible = geom.visible_rows(f64::from(self.obj().height())).count() as u32;
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

        /// Outline the ranges the formula being edited mentions, in the colours the text
        /// is coloured with — which is the whole point: the same scanner assigns both, so
        /// a reference and its outline cannot disagree about what it covers.
        fn draw_references(&self, f: &Frame) {
            if !self.mode.get().is_editing() {
                return;
            }
            let app = self.app.borrow().clone();
            let Some(app) = app else { return };
            let text = self.buffer.text().to_string();
            let pending = self.pending.borrow().clone();
            for (range, color) in crate::theme::reference_colors(&text, crate::theme::is_dark(&f.palette)) {
                let Ok(reference) = sheet_core::a1::parse(&text[range.clone()]) else {
                    continue;
                };
                let Ok((sheet, start, end)) = sheet_core::a1::resolve(&app, &reference) else {
                    continue;
                };
                if sheet != self.sheet.get() {
                    continue;
                }
                let top_left = f.geom.cell_rect(start.row, start.col);
                let bottom_right = f.geom.cell_rect(end.row, end.col);
                // The one being pointed at is thicker, so it is obvious which one an arrow
                // key will move.
                let thickness = match &pending {
                    Some(pending) if pending.span == range => 3.0,
                    _ => 2.0,
                };
                outline(
                    f.snapshot,
                    Rect {
                        x: top_left.x,
                        y: top_left.y,
                        w: bottom_right.x + bottom_right.w - top_left.x,
                        h: bottom_right.y + bottom_right.h - top_left.y,
                    },
                    color,
                    thickness,
                );
            }
        }

        /// The active cell's border, drawn after the text so it is never painted over.
        fn draw_active(&self, f: &Frame) {
            let cell = f.geom.cell_rect(f.selection.active.row, f.selection.active.col);
            outline(f.snapshot, cell, f.palette.accent, 2.0);
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

        /// A cell's `fo:background-color`, and the reason it is a pass of its own: an *empty*
        /// cell can be filled too, so this cannot ride along with the text.
        fn draw_backgrounds(&self, f: &Frame) {
            let Some(cells) = &f.cells else { return };
            for row in f.rows.clone() {
                for col in f.cols.clone() {
                    let Some(fill) = cells.style(row, col).and_then(|s| s.background.as_deref())
                    else {
                        continue;
                    };
                    // ODF's `"transparent"` is a colour that means "do not paint", and GDK
                    // parses it as opaque black — which would be a very visible bug.
                    let Some(color) = crate::theme::color(fill) else {
                        continue;
                    };
                    let cell = f.geom.cell_rect(row, col);
                    f.snapshot
                        .append_color(&color, &rect(cell.x, cell.y, cell.w, cell.h));
                }
            }
        }

        /// The four `fo:border-*` edges, over the grid lines.
        ///
        /// ponytail: the line style is ignored — `dashed` and `double` draw as solid. Dashes
        /// need `gsk::Stroke` and a path per edge, and nothing in the corpus renders visibly
        /// wrong without them. The width and colour are honoured, which is what carries the
        /// meaning of a ruled table.
        fn draw_borders(&self, f: &Frame) {
            let Some(cells) = &f.cells else { return };
            for row in f.rows.clone() {
                for col in f.cols.clone() {
                    let Some(style) = cells.style(row, col) else {
                        continue;
                    };
                    let cell = f.geom.cell_rect(row, col);
                    for (edge, border) in style.borders.iter().enumerate() {
                        let Some((points, _, color)) =
                            border.as_deref().and_then(sheet_core::style::border_parts)
                        else {
                            continue;
                        };
                        let Some(color) = crate::theme::color(color) else {
                            continue;
                        };
                        // A hairline is still a line: a 0.5pt border must not round to zero.
                        let t = (points * 4.0 / 3.0).max(1.0);
                        // `style::EDGES` order — left, right, top, bottom.
                        let edge = match edge {
                            0 => rect(cell.x, cell.y, t, cell.h),
                            1 => rect(cell.x + cell.w - t, cell.y, t, cell.h),
                            2 => rect(cell.x, cell.y, cell.w, t),
                            _ => rect(cell.x, cell.y + cell.h - t, cell.w, t),
                        };
                        f.snapshot.append_color(&color, &edge);
                    }
                }
            }
        }

        /// Draw the values, with the two overflow rules and the cell's own text styling.
        ///
        /// Columns were fetched with a margin either side so that a label anchored just
        /// off-screen still reaches into the view, and so that "is the next cell empty"
        /// can be answered without a second read.
        fn draw_cells(&self, f: &Frame) {
            let (geom, palette, rows) = (&f.geom, &f.palette, &f.rows);
            let Some(viewport) = &f.cells else { return };
            let fetch = viewport.cols.clone();

            // The cell being edited is drawn by the editor child, not here — otherwise its
            // stored value shows through the text being typed over it.
            let editing = self.mode.get().is_editing().then(|| self.selection.get().active);
            let layout = self.layout();
            // The padding is a distance on screen like everything else here, so it zooms.
            let pad = PAD * self.zoom.get();
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
                    // The style decides the font and the colour, and may override where the
                    // text sits — but not the *fallbacks*, which stay the value's own rules.
                    let style = viewport.style(row, col);
                    layout.set_attributes(self.attrs(style.and_then(font)).as_ref());
                    let color = style
                        .and_then(|s| s.color.as_deref())
                        .and_then(crate::theme::color)
                        .unwrap_or(palette.foreground);
                    let align = style
                        .and_then(|s| s.align.as_deref())
                        .and_then(aligned)
                        .unwrap_or_else(|| alignment(value));
                    let valign = valigned(style.and_then(|s| s.vertical_align.as_deref()));
                    let wrapping = style.is_some_and(|s| s.wrap.as_deref() == Some("wrap"));

                    let cell = geom.cell_rect(row, col);
                    // A wrapped cell is drawn inside its own width, and its row was grown to
                    // fit what wraps into it (`measure_rows`) unless the document set that
                    // row a height of its own — an explicit height means explicit, so it
                    // still clips.
                    layout.set_width(match wrapping {
                        true => ((cell.w - 2.0 * pad).max(1.0) * f64::from(pango::SCALE)) as i32,
                        false => -1,
                    });
                    layout.set_text(text);
                    let (text_w, text_h) = layout.pixel_size();
                    let fits = wrapping || f64::from(text_w) <= cell.w - 2.0 * pad;

                    // A number that does not fit is never truncated — a wrong magnitude
                    // read as a right one is worse than no reading at all.
                    if !fits && align == Align::Right {
                        layout.set_text("##########");
                        let (w, h) = layout.pixel_size();
                        draw_text(f.snapshot, &layout, color, &cell, cell, w, h, (Align::Right, valign), pad);
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
                        color,
                        &cell,
                        paint,
                        text_w,
                        text_h,
                        (align, valign),
                        pad,
                    );
                }
            }
            // The layout is shared and reused, so anything set for one cell has to be unset
            // or the headers inherit it.
            layout.set_attributes(None);
            layout.set_width(-1);
        }

        fn draw_headers(&self, f: &Frame) {
            let (snapshot, geom, palette) = (f.snapshot, &f.geom, &f.palette);
            let (width, height, rows, cols) = (f.width, f.height, &f.rows, &f.cols);
            let layout = self.layout();
            layout.set_attributes(self.attrs(None).as_ref());
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
                draw_text(snapshot, &layout, palette.header_text, &head, head, w, h, (Align::Center, VAlign::Middle), PAD);
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
                draw_text(snapshot, &layout, palette.header_text, &head, head, w, h, (Align::Center, VAlign::Middle), PAD);
            }
            snapshot.pop();

            layout.set_attributes(None);
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

    /// Where a cell's text sits in it vertically. Middle unless the cell says otherwise —
    /// which is the same default `style:vertical-align="automatic"` means for a value.
    #[derive(Clone, Copy, PartialEq, Eq, Default)]
    enum VAlign {
        Top,
        #[default]
        Middle,
        Bottom,
    }

    /// `fo:text-align` as this grid draws it (§16.5, and `core/src/style.rs` keeps the ODF
    /// spelling verbatim). `start`/`end` are relative to the writing direction, and this grid
    /// is left-to-right; anything else — `justify`, a value from a newer ODF — falls back to
    /// the value's own rule rather than guessing.
    fn aligned(value: &str) -> Option<Align> {
        match value {
            "start" | "left" => Some(Align::Left),
            "center" => Some(Align::Center),
            "end" | "right" => Some(Align::Right),
            _ => None,
        }
    }

    fn valigned(value: Option<&str>) -> VAlign {
        match value {
            Some("top") => VAlign::Top,
            Some("bottom") => VAlign::Bottom,
            _ => VAlign::Middle,
        }
    }

    /// A cell style's font as Pango attributes, or `None` when it says nothing about one.
    ///
    /// Weight, slant and size only: `fo:font-family` is deliberately not carried by the model
    /// (LibreOffice rewrites it into a font-face reference, `core/src/style.rs`), so there is
    /// nothing here to set a family from.
    ///
    /// A size larger than the row's default grows the row rather than clipping — the tallest
    /// styled cell in a row is what [`imp::Grid::measure_rows`] takes its height from.
    fn font(style: &sheet_core::style::CellStyle) -> Option<pango::AttrList> {
        let attrs = pango::AttrList::new();
        let mut any = false;
        if let Some(weight) = style.font_weight.as_deref() {
            let weight = match weight {
                "bold" => Some(pango::Weight::Bold),
                "normal" => Some(pango::Weight::Normal),
                // ODF also allows a hundreds number, and that *is* Pango's scale — 700 is
                // `Bold` on both sides, so the number is passed through rather than mapped.
                n => n.parse::<i32>().ok().map(pango::Weight::__Unknown),
            };
            if let Some(weight) = weight {
                attrs.insert(pango::AttrInt::new_weight(weight));
                any = true;
            }
        }
        if let Some(slant) = style.font_style.as_deref() {
            let slant = match slant {
                "italic" => Some(pango::Style::Italic),
                "oblique" => Some(pango::Style::Oblique),
                "normal" => Some(pango::Style::Normal),
                _ => None,
            };
            if let Some(slant) = slant {
                attrs.insert(pango::AttrInt::new_style(slant));
                any = true;
            }
        }
        // A length in points, which is what a spreadsheet's font size always is. A
        // percentage — ODF allows one — is relative to a style this model does not inherit
        // from, so it is left alone rather than resolved against the wrong base.
        if let Some(points) = style
            .font_size
            .as_deref()
            .and_then(|size| size.strip_suffix("pt"))
            .and_then(|points| points.parse::<f64>().ok())
        {
            attrs.insert(pango::AttrSize::new((points * f64::from(pango::SCALE)) as i32));
            any = true;
        }
        any.then_some(attrs)
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
        align: (Align, VAlign),
        pad: f64,
    ) {
        let (align, valign) = align;
        let text_w = f64::from(text_w);
        let text_h = f64::from(text_h);
        let x = match align {
            Align::Left => cell.x + pad,
            Align::Center => cell.x + (cell.w - text_w) / 2.0,
            Align::Right => cell.x + cell.w - pad - text_w,
        };
        let y = match valign {
            VAlign::Top => cell.y + pad / 2.0,
            VAlign::Middle => cell.y + (cell.h - text_h) / 2.0,
            VAlign::Bottom => cell.y + cell.h - pad / 2.0 - text_h,
        };
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
            K::F4 => Key::F4,
            other => other.to_unicode().map_or(Key::Other, Key::Char),
        }
    }

    /// A rectangle drawn as four thin ones, since a snapshot has no stroke.
    fn outline(snapshot: &gtk::Snapshot, r: Rect, color: gtk::gdk::RGBA, t: f64) {
        for edge in [
            rect(r.x - 1.0, r.y - 1.0, r.w + 2.0, t),
            rect(r.x - 1.0, r.y + r.h - 1.0, r.w + 2.0, t),
            rect(r.x - 1.0, r.y - 1.0, t, r.h + 2.0),
            rect(r.x + r.w - 1.0, r.y - 1.0, t, r.h + 2.0),
        ] {
            snapshot.append_color(&color, &edge);
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
