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

use grind_sheet::{App, Pos, a1};
use gtk::{gio, glib};
use libadwaita::subclass::prelude::ObjectSubclassIsExt;

use crate::geom::{GridGeom, HANDLE, MAX_COLS, MAX_ROWS, Sizes};
use crate::keymap::{Dir, Selection};

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

/// The smallest a chart may be resized to, in widget pixels — small enough not to fight a
/// deliberate shrink, large enough that the resize handle sitting on its own corner is never
/// bigger than the chart itself.
const MIN_CHART: f64 = 24.0;

/// Below this many pixels of total offset, a press-and-release on a chart's body is a click
/// rather than a move — the colour popover's own gesture, distinguished from a drag the same
/// way [`gtk::GestureClick`] distinguishes a click from the start of a drag elsewhere in this
/// widget.
const CHART_CLICK_THRESHOLD: f64 = 4.0;

/// What a filter button is marked with. A glyph rather than a drawn triangle: the layout is
/// already here for every other piece of text in the grid, and a path builder for one arrow
/// is a second way of drawing.
const CHEVRON: &str = "\u{25be}";

/// The bar under a filtered field's chevron, in pixels.
const UNDERLINE: f64 = 2.0;

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

    /// Which of `doc/view-modes.md`'s overlays this view draws.
    pub fn overlays(&self) -> grind_sheet::view::Overlays {
        self.imp().overlays.get()
    }

    /// Draw the overlays, or stop drawing them — a **reading** of the document and never a
    /// write, so this changes nothing but the paint. Toggling one back off restores exactly
    /// what was on screen before, because nothing was stored in the first place.
    pub fn set_overlays(&self, overlays: grind_sheet::view::Overlays) {
        self.imp().overlays.set(overlays);
        self.queue_draw();
        for hook in self.imp().on_overlays.borrow().iter() {
            hook(overlays);
        }
    }

    /// Told whenever an overlay is switched on or off — the status bar's indication that
    /// role mode is on, which `doc/view-modes.md` §9 asks for by name.
    pub fn connect_overlays_changed(&self, f: impl Fn(grind_sheet::view::Overlays) + 'static) {
        self.imp().on_overlays.borrow_mut().push(Box::new(f));
    }

    pub fn zoom(&self) -> f64 {
        self.imp().zoom.get()
    }

    /// Scale the view. Clamped to [`ZOOM_RANGE`]; a factor of 1 is the document at the
    /// toolkit's own idea of its size. Anchored on the centre of the view — the keyboard's
    /// zoom; the wheel and a pinch anchor on the pointer instead (`imp::Grid::rezoom`).
    pub fn set_zoom(&self, zoom: f64) {
        self.imp().rezoom(zoom, None);
    }

    /// Told whenever the zoom changes, with the new factor — the status bar's readout.
    pub fn connect_zoom_changed(&self, f: impl Fn(f64) + 'static) {
        self.imp().on_zoom.borrow_mut().push(Box::new(f));
    }

    /// Every used column autofit to its widest text and every explicit row height cleared,
    /// so the whole sheet fits what is in it — the bulk form of double-clicking each column
    /// and row boundary in turn.
    pub fn autofit_all(&self) {
        self.imp().autofit_all();
    }

    /// The selection's calculated values on the clipboard, rather than its formulas.
    pub fn copy_value(&self) {
        self.imp().copy_value();
    }

    /// Fill down or right — the toolbar's twin of Ctrl+D / Ctrl+R.
    pub fn fill(&self, dir: Dir) {
        self.imp().fill(dir);
    }

    /// The clipboard four, as `win.` actions rather than only as keys.
    ///
    /// They existed here already — [`crate::keymap::Action`] has reached them since M5 — but only
    /// from the keyboard, which meant the context menu `doc/sheet-shell.md`'s rework adds had
    /// nothing to point at. A `win.` action is the one spelling a menu item, an accelerator
    /// and a palette row can all share.
    pub fn copy(&self, cut: bool) {
        self.imp().copy(cut);
    }

    pub fn paste(&self) {
        self.imp().paste();
    }

    /// Clear what is selected — Delete's twin, one undo step (`App::clear_range`).
    pub fn clear(&self) {
        self.imp().clear();
    }

    /// Select the used extent — Ctrl+A's twin.
    pub fn select_all(&self) {
        let (rows, cols) = self.imp().used_extent();
        self.set_selection(Selection {
            anchor: Pos::new(0, 0),
            active: Pos::new(rows.saturating_sub(1), cols.saturating_sub(1)),
        });
    }

    /// Filter the selection, or clear the filter the sheet already has (§9.4) — the
    /// toolbar's twin of `sheet filter <range>` and `sheet filter --clear`.
    pub fn toggle_filter(&self) {
        self.imp().toggle_filter();
    }

    /// Whether the sheet has an autofilter, which is what the toolbar button labels itself
    /// from: the same button clears one and creates one, so it has to say which.
    pub fn has_filter(&self) -> bool {
        self.imp().filter().is_some()
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
    /// The user asked to change the chart at this index on the active sheet — a double-click
    /// on it, or *Edit Chart* from its context menu. The dialog lives in the window rather
    /// than here, so the grid says what was asked for instead of asking it.
    EditChart(usize),
    /// The user asked to delete that chart, from the same menu.
    DeleteChart(usize),
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
        // Never actually reached: `Grid::press` unhides on this hit and returns before a
        // selection is ever asked for. Kept total anyway, the same shape as a header's.
        Hit::HiddenCols(from, _) => Selection {
            anchor: Pos::new(MAX_ROWS - 1, from),
            active: Pos::new(0, from),
        },
        Hit::HiddenRows(from, _) => Selection {
            anchor: Pos::new(from, MAX_COLS - 1),
            active: Pos::new(from, 0),
        },
        Hit::Corner => Selection {
            anchor: Pos::new(MAX_ROWS - 1, MAX_COLS - 1),
            active: Pos::new(0, 0),
        },
    }
}

/// What a right-click on a header hides: `is_cols`, and the half-open run. Not a header at
/// all gives `None`, which is the whole gate on the menu appearing. Free of the widget for
/// the same reason [`selection_for_hit`] is.
///
/// A header inside `selection` hides the *whole* selected run — several headers dragged
/// over and then hidden in one go is the ordinary gesture — and one outside it hides just
/// itself, the same "click somewhere else, act on just that" rule the fill handle and the
/// filter button already follow.
fn hide_range_for_hit(selection: Selection, hit: crate::geom::Hit) -> Option<(bool, u32, u32)> {
    use crate::geom::Hit;
    let (start, end) = selection.rect();
    match hit {
        Hit::ColHeader(col) | Hit::ColEdge(col) => {
            let whole = start.row == 0 && end.row == MAX_ROWS - 1;
            match whole && (start.col..=end.col).contains(&col) {
                true => Some((true, start.col, end.col + 1)),
                false => Some((true, col, col + 1)),
            }
        }
        Hit::RowHeader(row) | Hit::RowEdge(row) => {
            let whole = start.col == 0 && end.col == MAX_COLS - 1;
            match whole && (start.row..=end.row).contains(&row) {
                true => Some((false, start.row, end.row + 1)),
                false => Some((false, row, row + 1)),
            }
        }
        _ => None,
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
        assert_eq!(
            all.rect(),
            (Pos::new(0, 0), Pos::new(MAX_ROWS - 1, MAX_COLS - 1))
        );
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

    /// Right-clicking a header outside the current selection hides just that one track.
    #[test]
    fn right_clicking_an_unselected_header_hides_only_it() {
        let selection = Selection::at(Pos::new(0, 0));
        assert_eq!(
            hide_range_for_hit(selection, Hit::ColHeader(3)),
            Some((true, 3, 4))
        );
        assert_eq!(
            hide_range_for_hit(selection, Hit::RowHeader(5)),
            Some((false, 5, 6))
        );
    }

    /// Right-clicking a header that is part of a multi-track selection hides the whole
    /// selected run in one step, not just the header clicked.
    #[test]
    fn right_clicking_a_selected_header_hides_the_whole_run() {
        let selection = Selection {
            anchor: selection_for_hit(Hit::ColHeader(2)).anchor,
            active: selection_for_hit(Hit::ColHeader(5)).active,
        };
        assert_eq!(
            hide_range_for_hit(selection, Hit::ColHeader(4)),
            Some((true, 2, 6)),
            "col 4 is inside the 2..=5 selection"
        );
        assert_eq!(
            hide_range_for_hit(selection, Hit::ColHeader(9)),
            Some((true, 9, 10)),
            "col 9 is outside it, so only col 9 is hidden"
        );
    }

    /// Right-clicking anything other than a header does not offer to hide anything.
    #[test]
    fn right_clicking_a_cell_offers_nothing_to_hide() {
        let selection = Selection::at(Pos::new(0, 0));
        assert_eq!(
            hide_range_for_hit(selection, Hit::Cell { row: 1, col: 1 }),
            None
        );
    }
}

/// The right-click menu over the cells (`doc/sheet-shell.md`, "Four surfaces").
///
/// **Where a verb about the selection goes.** Every item is a `win.` action, which resolves up
/// the widget tree to the window — so the grid lists these verbs and owns none of them, and an
/// item cannot appear here that the palette and the shortcuts window do not also know about.
/// Three sections, in the order a person reaches for them: what to do with the cells' content,
/// what to do to the cells, and what the *range* is rather than what is in it — those last
/// three being the ones that used to sit on the tool strip's Calculate page, where they were
/// always about a selection and never about calculating.
///
/// A free function returning the model rather than one built inline, so that `main.rs`'s
/// ratchet can walk it: an item naming an action nobody declares is silent at runtime.
pub fn cell_menu_model() -> gio::Menu {
    let model = gio::Menu::new();
    let clipboard = gio::Menu::new();
    clipboard.append(Some("Cut"), Some("win.cut"));
    clipboard.append(Some("Copy"), Some("win.copy"));
    clipboard.append(Some("Copy Value"), Some("win.copy-value"));
    clipboard.append(Some("Paste"), Some("win.paste"));
    model.append_section(None, &clipboard);

    let edits = gio::Menu::new();
    edits.append(Some("Clear Contents"), Some("win.clear"));
    edits.append(Some("Fill Down"), Some("win.fill-down"));
    edits.append(Some("Fill Right"), Some("win.fill-right"));
    model.append_section(None, &edits);

    let range = gio::Menu::new();
    range.append(Some("Name This Range…"), Some("win.names"));
    range.append(Some("Filter Rows"), Some("win.filter"));
    range.append(Some("Insert Chart…"), Some("win.chart-insert"));
    model.append_section(None, &range);
    model
}

mod imp {
    use super::*;

    use grind_sheet::formula::value::FormulaError;
    use grind_sheet::{CellValue, Pos};
    use gtk::graphene;
    use gtk::gsk;
    use gtk::pango;
    use gtk::subclass::prelude::*;

    use grind_sheet::RecalcMode;
    use grind_sheet::formula::display;
    use grind_sheet::style;

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
    type ZoomHook = Box<dyn Fn(f64)>;
    type OverlaysHook = Box<dyn Fn(grind_sheet::view::Overlays)>;

    /// Space either side of a cell's text.
    const PAD: f64 = 4.0;
    /// The width reserved at a cell's leading edge for `doc/view-modes.md`'s role marker —
    /// §4.6's second channel, so the mode is usable with no colour discrimination at all.
    /// A distance on screen, so it zooms with everything else.
    const ROLE_MARK: f64 = 9.0;
    /// Slack added on top of a measured autofit width. The same text gets measured twice —
    /// once unzoomed here, once at zoom when it is actually drawn — and Pango's own
    /// sub-pixel rounding does not round the same way both times, so a width fit exactly to
    /// the first measurement can be a pixel short of the second and trip the overflow rule.
    const FIT_SLACK: f64 = 2.0;
    /// Space above and below it, which is what makes the default row taller than a line.
    const ROW_PAD: f64 = 8.0;
    /// How much sheet is measured for natural row heights. A row above the view still
    /// displaces the ones below it, so this pass cannot be limited to what is on screen —
    /// past this much document every row keeps the default height instead.
    const AUTO_HEIGHT_CELLS: u64 = 200_000;
    /// How many columns are fetched beyond the visible ones, so that a label anchored just
    /// off-screen still overflows into view.
    const OVERFLOW_MARGIN: u32 = 12;
    /// How far a programmatic scroll may be, in default row heights, before it glides
    /// there instead of teleporting. Below this — an arrow key nudging the view a row —
    /// it stays instant, which is what a keyboard repeat rate needs.
    const GLIDE_AFTER: f64 = 3.0;
    /// How long the glide takes. Short enough that holding an arrow key never waits on it.
    const GLIDE_MS: u32 = 150;

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
        cells: Option<grind_sheet::Viewport>,
        /// The sheet's autofilter (§9.4), read once for the same reason as `cells`: the
        /// buttons pass asks about it per visible column, and that is one lock, not twelve.
        filter: Option<grind_sheet::Filter>,
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
        /// Which of `doc/view-modes.md`'s overlays this view is drawing — a *reading* of
        /// the document, never a change to it, so it belongs with the presentation state
        /// rather than in the core's document.
        pub overlays: Cell<grind_sheet::view::Overlays>,
        pub on_overlays: RefCell<Vec<OverlaysHook>>,
        /// Presentation state, and the only state this widget has.
        pub selection: Cell<Selection>,
        /// What a drag started on, so that dragging across headers selects whole columns
        /// rather than the cells the pointer happens to pass over.
        pub drag: Cell<Option<Hit>>,
        /// A track being resized, in pixels — presentation state until the pointer is
        /// released, at which point it becomes one core write and one undo entry.
        pub resize: Cell<Option<Resize>>,
        /// A chart being moved or resized, and which chart it is.
        pub chart_drag: Cell<Option<ChartDrag>>,
        /// Where that drag currently puts the chart, in widget space — painted in its place
        /// until the pointer is released and it becomes `App::reshape_chart`.
        pub chart_drag_rect: Cell<Option<Rect>>,
        /// Whether the drag in progress started on the fill handle.
        pub filling: Cell<bool>,
        /// Where that drag is pointing ([`keymap::fill_target`]), painted as an outline
        /// until the pointer is released and it becomes the fill.
        pub fill_to: Cell<Option<(Dir, u32)>>,
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
        /// Where the pointer last was, in widget coordinates — what Ctrl+wheel zooms
        /// around, so the cell under the cursor stays under the cursor.
        pub pointer: Cell<Option<(f64, f64)>>,
        /// The zoom when a pinch began; the gesture reports a scale relative to that.
        pub pinch_base: Cell<f64>,
        /// A programmatic scroll mid-glide, so a newer jump can stop it.
        pub glide: RefCell<Option<libadwaita::TimedAnimation>>,
        pub on_zoom: RefCell<Vec<ZoomHook>>,
        /// The autocomplete popover, parented to this widget so it appears under the cell
        /// being typed into rather than at the formula bar.
        pub completion: OnceCell<crate::formula_ux::Completion>,
        /// The autofilter dropdown (§9.4), parented here for the same reason: it belongs
        /// under the header cell's button, which only this widget knows the position of.
        pub filter_menu: OnceCell<std::rc::Rc<crate::filter_ui::FilterMenu>>,
        /// The right-click "Hide" menu for a column or row header (§5.4) — one button, built
        /// once and relabelled/repositioned on each open rather than rebuilt, the same
        /// reuse `filter_menu` gets for a heavier reason.
        pub hide_menu: OnceCell<gtk::Popover>,
        pub hide_button: OnceCell<gtk::Button>,
        /// The right-click menu over a chart — *Edit* and *Delete*, built once and
        /// repositioned per opening, exactly as `hide_menu` is.
        pub chart_menu: OnceCell<gtk::Popover>,
        /// The right-click menu over the **cells** (`doc/sheet-shell.md`'s four surfaces):
        /// what a person can do to the selection, where the selection is. Built from a
        /// `gio::Menu` of `win.` actions rather than by hand, so the same verbs the palette
        /// and the keyboard reach are the ones listed here.
        pub cell_menu: OnceCell<gtk::PopoverMenu>,
        /// Which chart that menu is currently open over.
        pub chart_menu_target: Cell<Option<usize>>,
        /// What the menu is currently open over: columns or rows, and the half-open range
        /// its button hides — read back when the button is clicked, since the click handler
        /// is wired once rather than once per opening.
        pub hide_target: Cell<Option<(bool, u32, u32)>>,
    }

    /// A column or row being dragged wider.
    #[derive(Clone, Copy, Debug)]
    pub struct Resize {
        /// [`Hit::ColEdge`] or [`Hit::RowEdge`] — which boundary was grabbed.
        pub track: Hit,
        pub size: f64,
    }

    /// A chart being moved or resized by hand — presentation state until the pointer is
    /// released, the same as [`Resize`] and the fill handle are. The live rect this produces
    /// is painted from instead of the document's own geometry (`Grid::draw_charts`), which is
    /// what makes the drag itself smooth rather than one written cell per pixel of motion —
    /// this shell's whole answer to "I hate how dragging a chart feels in LibreOffice."
    #[derive(Clone, Copy, Debug)]
    pub enum ChartDrag {
        /// Moving: the offset from the chart's own top-left corner to the point the pointer
        /// grabbed it at, so the chart does not jump to be centred under the pointer the
        /// moment the drag starts.
        Move {
            index: usize,
            grab_dx: f64,
            grab_dy: f64,
        },
        /// Resizing from the bottom-right handle: the chart's own top-left corner, fixed for
        /// the whole drag since only the far corner is moving.
        Resize {
            index: usize,
            origin_x: f64,
            origin_y: f64,
        },
    }

    impl ChartDrag {
        fn index(self) -> usize {
            match self {
                ChartDrag::Move { index, .. } | ChartDrag::Resize { index, .. } => index,
            }
        }
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
                overlays: Cell::new(grind_sheet::view::Overlays::NONE),
                on_overlays: RefCell::new(Vec::new()),
                selection: Cell::new(Selection::default()),
                drag: Cell::new(None),
                resize: Cell::new(None),
                chart_drag: Cell::new(None),
                chart_drag_rect: Cell::new(None),
                filling: Cell::new(false),
                fill_to: Cell::new(None),
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
                pointer: Cell::new(None),
                pinch_base: Cell::new(1.0),
                glide: RefCell::new(None),
                on_zoom: RefCell::new(Vec::new()),
                completion: OnceCell::new(),
                filter_menu: OnceCell::new(),
                hide_menu: OnceCell::new(),
                hide_button: OnceCell::new(),
                chart_menu: OnceCell::new(),
                chart_menu_target: Cell::new(None),
                cell_menu: OnceCell::new(),
                hide_target: Cell::new(None),
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
                "hadjustment" => {
                    self.set_adjustment(gtk::Orientation::Horizontal, value.get().ok())
                }
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
            if let Some(menu) = self.filter_menu.get() {
                menu.dispose();
            }
            if let Some(menu) = self.hide_menu.get() {
                menu.unparent();
            }
            if let Some(menu) = self.chart_menu.get() {
                menu.unparent();
            }
            if let Some(menu) = self.cell_menu.get() {
                menu.unparent();
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

            // The dropdown decides which values to keep; turning that into an undoable
            // change is this widget's job, because it is the one holding the document.
            let menu = crate::filter_ui::FilterMenu::new(&*widget);
            menu.connect_apply(glib::clone!(
                #[weak(rename_to = grid)]
                widget,
                move |field, chosen| grid.imp().apply_filter(field, chosen)
            ));
            let _ = self.filter_menu.set(menu);

            // The right-click "Hide" menu (§5.4): one flat button, relabelled and
            // repositioned per opening rather than rebuilt — `hide_target` is what the
            // click handler, wired once here, reads to know which run it is hiding.
            let hide_button = gtk::Button::new();
            hide_button.add_css_class("flat");
            hide_button.connect_clicked(glib::clone!(
                #[weak(rename_to = grid)]
                widget,
                move |_| {
                    let imp = grid.imp();
                    if let Some((is_cols, from, to)) = imp.hide_target.take() {
                        match is_cols {
                            true => imp.hide_cols(from, to),
                            false => imp.hide_rows(from, to),
                        }
                    }
                    if let Some(menu) = imp.hide_menu.get() {
                        menu.popdown();
                    }
                }
            ));
            let hide_menu = gtk::Popover::builder()
                .child(&hide_button)
                .has_arrow(true)
                .position(gtk::PositionType::Bottom)
                .build();
            hide_menu.set_parent(&*widget);
            let _ = self.hide_button.set(hide_button);
            let _ = self.hide_menu.set(hide_menu);

            // The right-click menu over a chart. Both items hand the window a `Notice` rather
            // than doing the work here: a dialog and an undo toast are the window's, and the
            // grid's job is to say which chart was asked about.
            let chart_items = gtk::Box::new(gtk::Orientation::Vertical, 0);
            for (label, notice) in [
                ("Edit Chart…", Notice::EditChart as fn(usize) -> Notice),
                ("Delete Chart", Notice::DeleteChart as fn(usize) -> Notice),
            ] {
                let button = gtk::Button::with_label(label);
                button.add_css_class("flat");
                button.connect_clicked(glib::clone!(
                    #[weak(rename_to = grid)]
                    widget,
                    move |_| {
                        let imp = grid.imp();
                        if let Some(index) = imp.chart_menu_target.take() {
                            imp.notice(notice(index));
                        }
                        if let Some(menu) = imp.chart_menu.get() {
                            menu.popdown();
                        }
                    }
                ));
                chart_items.append(&button);
            }
            let chart_menu = gtk::Popover::builder()
                .child(&chart_items)
                .has_arrow(true)
                .position(gtk::PositionType::Bottom)
                .build();
            chart_menu.set_parent(&*widget);
            let _ = self.chart_menu.set(chart_menu);

            let cell_menu = gtk::PopoverMenu::from_model(Some(&super::cell_menu_model()));
            cell_menu.set_parent(&*widget);
            cell_menu.set_has_arrow(false);
            let _ = self.cell_menu.set(cell_menu);

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
                    // A chart is over the cells, so a double-click on one means the chart —
                    // the same "open what was double-clicked" every object in a spreadsheet
                    // has, and the reason the cell editor never opens underneath it.
                    if let Some((index, _, _)) = grid.imp().chart_hit(x, y) {
                        grid.imp().notice(Notice::EditChart(index));
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
                move |_, offset_x, offset_y| {
                    grid.imp().drag.set(None);
                    grid.imp().commit_resize();
                    grid.imp().commit_fill();
                    grid.imp().commit_chart_drag(offset_x, offset_y);
                }
            ));
            widget.add_controller(drag);

            // Right-click, in the order the things under the pointer overlap: a chart floats
            // over the cells, a header means "hide" and nothing else (§5.4), and a cell means
            // the selection it is part of.
            let secondary = gtk::GestureClick::new();
            secondary.set_button(gtk::gdk::BUTTON_SECONDARY);
            secondary.connect_pressed(glib::clone!(
                #[weak(rename_to = grid)]
                widget,
                move |_, _, x, y| {
                    let imp = grid.imp();
                    if let Some((index, _, _)) = imp.chart_hit(x, y) {
                        imp.open_chart_menu(index, x, y);
                        return;
                    }
                    let hit = imp.geom().hit(x, y);
                    let selection = imp.selection.get();
                    if let Some((is_cols, from, to)) = hide_range_for_hit(selection, hit) {
                        imp.open_hide_menu(x, y, is_cols, from, to);
                        return;
                    }
                    if let Hit::Cell { row, col } = hit {
                        imp.open_cell_menu(Pos::new(row, col), x, y);
                    }
                }
            ));
            widget.add_controller(secondary);

            // The pointer says what a press would do before it happens, which is the only
            // thing that makes a 4px target discoverable. Its position is also remembered,
            // because that is what a Ctrl+wheel zoom anchors on.
            let motion = gtk::EventControllerMotion::new();
            motion.connect_motion(glib::clone!(
                #[weak(rename_to = grid)]
                widget,
                move |_, x, y| {
                    grid.imp().pointer.set(Some((x, y)));
                    let geom = grid.imp().geom();
                    let (_, corner) = grid.imp().selection.get().rect();
                    let hit = geom.hit(x, y);
                    let cursor = match hit {
                        Hit::ColEdge(_) => Some("col-resize"),
                        Hit::RowEdge(_) => Some("row-resize"),
                        Hit::HiddenCols(_, _) | Hit::HiddenRows(_, _) => Some("pointer"),
                        _ if geom.fill_handle(corner.row, corner.col).contains(x, y) => {
                            Some("crosshair")
                        }
                        // A 14px control nobody has been told about needs the cursor to say
                        // it is one, exactly like the resize boundaries above.
                        _ if grid.imp().filter_button_under(&geom, hit, x, y).is_some() => {
                            Some("pointer")
                        }
                        _ => None,
                    };
                    grid.set_cursor_from_name(cursor);
                }
            ));
            motion.connect_leave(glib::clone!(
                #[weak(rename_to = grid)]
                widget,
                move |_| grid.imp().pointer.set(None)
            ));
            widget.add_controller(motion);

            // Ctrl+wheel zooms, anchored on the pointer — the cell under the cursor stays
            // under the cursor. Every other wheel event travels on to the scrolled window,
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
                    let imp = grid.imp();
                    imp.rezoom(grid.zoom() * ZOOM_STEP.powf(-dy), imp.pointer.get());
                    glib::Propagation::Stop
                }
            ));
            widget.add_controller(scroll);

            // A touchpad or touchscreen pinch is the same zoom, anchored on the gesture's
            // own centre. The gesture needs two touch points, so it cannot collide with the
            // click and drag gestures above.
            // Dark mode and the accent colour arrive as style-manager properties, not
            // system settings, and GTK does not expose the `css_changed` vfunc — without
            // these two, flipping either at runtime would leave the grid painted from the
            // stale cached palette until restart. The accent property is libadwaita ≥ 1.6;
            // connecting to its notify by name is inert on 1.5 rather than an error.
            let style = libadwaita::StyleManager::default();
            style.connect_dark_notify(glib::clone!(
                #[weak(rename_to = grid)]
                widget,
                move |_| grid.imp().restyle()
            ));
            style.connect_notify_local(
                Some("accent-color"),
                glib::clone!(
                    #[weak(rename_to = grid)]
                    widget,
                    move |_, _| grid.imp().restyle()
                ),
            );

            let pinch = gtk::GestureZoom::new();
            pinch.connect_begin(glib::clone!(
                #[weak(rename_to = grid)]
                widget,
                move |_, _| grid.imp().pinch_base.set(grid.zoom())
            ));
            pinch.connect_scale_changed(glib::clone!(
                #[weak(rename_to = grid)]
                widget,
                move |gesture, scale| {
                    let imp = grid.imp();
                    imp.rezoom(imp.pinch_base.get() * scale, gesture.bounding_box_center());
                }
            ));
            widget.add_controller(pinch);
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
                &gtk::gdk::Rectangle::new(
                    rect.x as i32,
                    rect.y as i32,
                    rect.w as i32,
                    rect.h as i32,
                ),
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
                filter: self.filter(),
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
            // The role marks go over the values for the same reason the hints do: both are
            // drawn in the room `draw_cells` left them.
            self.draw_roles(&frame);
            // Over the values, because a hint is drawn in the room the value left and has
            // to be able to sit on the space an overflowing neighbour would otherwise use.
            self.draw_hints(&frame);
            // Charts float over the sheet body (`table:shapes` is a sibling of the rows, not
            // inside one), so they are drawn over every cell's own text.
            self.draw_charts(&frame);
            // Over the text, because the button sits on top of the heading's right-hand end
            // and the heading is what would otherwise run through it.
            self.draw_filter_buttons(&frame);
            self.draw_active(&frame);
            self.draw_fill(&frame);
            self.draw_references(&frame);
            snapshot.pop();

            self.draw_headers(&frame);
            self.draw_hidden_markers(&frame);
            self.draw_resize_hint(&frame);
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
            let mut widths = self.track_lengths(false);
            // Last, so a column hidden by hand is hidden whatever width it was given: zero,
            // which `Sizes` keeps as a track that displaces nothing — the column twin of
            // what filtering already does to a row's height in `geom` below.
            let sheet = self.sheet.get();
            if let Some(app) = self.app.borrow().as_ref() {
                widths.extend(
                    app.hidden_cols(sheet)
                        .unwrap_or_default()
                        .into_iter()
                        .map(|col| (col, 0.0)),
                );
            }
            Sizes::new(self.metrics.get().col_width, MAX_COLS, widths)
        }

        fn geom(&self) -> GridGeom {
            let m = self.metrics.get();
            let zoom = self.zoom.get();
            // The document's own heights are given *after* the measured ones, because
            // `Sizes::new` keeps the last entry for an index: a row the document sized keeps
            // that size and clips, which is what an explicit height means.
            let mut heights = self.auto_heights();
            heights.extend(self.track_lengths(true));
            // Last, so a filtered or manually hidden row is hidden whatever height it was
            // given: zero, which `Sizes` keeps as a track that displaces nothing.
            let sheet = self.sheet.get();
            if let Some(app) = self.app.borrow().as_ref() {
                heights.extend(
                    app.hidden_rows(sheet)
                        .unwrap_or_default()
                        .into_iter()
                        .map(|row| (row, 0.0)),
                );
                heights.extend(
                    app.manually_hidden_rows(sheet)
                        .unwrap_or_default()
                        .into_iter()
                        .map(|row| (row, 0.0)),
                );
            }
            let mut rows = Sizes::new(m.row_height, MAX_ROWS, heights).scaled(zoom);
            let mut cols = self.col_sizes().scaled(zoom);
            // A resize in progress is painted before it is stored, so the whole grid reflows
            // under the pointer rather than a guide line standing in for it. It arrives in
            // screen pixels, which is why it goes on after the zoom rather than before.
            match self.resize.get() {
                Some(Resize {
                    track: Hit::ColEdge(col),
                    size,
                }) => cols = cols.with(col, size),
                Some(Resize {
                    track: Hit::RowEdge(row),
                    size,
                }) => rows = rows.with(row, size),
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
            if used_rows == 0
                || used_cols == 0
                || u64::from(used_rows) * u64::from(used_cols) > AUTO_HEIGHT_CELLS
            {
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
                        true => {
                            ((cols.size_of(col) - 2.0 * PAD).max(1.0) * f64::from(pango::SCALE))
                                as i32
                        }
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
        ) -> Option<grind_sheet::Viewport> {
            let app = self.app.borrow();
            let app = app.as_ref()?;
            let fetch = cols.start.saturating_sub(OVERFLOW_MARGIN)
                ..(cols.end.saturating_add(OVERFLOW_MARGIN)).min(MAX_COLS);
            app.get_viewport_with(self.sheet.get(), rows.clone(), fetch, self.overlays.get())
                .ok()
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
            // The reference colouring picks its palette by light or dark, so an edit open
            // across a theme flip has to be recoloured with the new one.
            if self.mode.get().is_editing() {
                self.restyle_formula();
            }
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

        fn set_adjustment(
            &self,
            orientation: gtk::Orientation,
            adjustment: Option<gtk::Adjustment>,
        ) {
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

        pub fn used_extent(&self) -> (u32, u32) {
            self.app
                .borrow()
                .as_ref()
                .and_then(|app| app.used_extent(self.sheet.get()).ok())
                .unwrap_or((0, 0))
        }

        // --- events ---

        /// A key, in [`keymap`]'s vocabulary. `Proceed` for everything this shell does not
        /// claim, which is what keeps the toolkit's own bindings working.
        fn key_pressed(
            &self,
            keyval: gtk::gdk::Key,
            state: gtk::gdk::ModifierType,
        ) -> glib::Propagation {
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
                    keymap::moved(
                        self.selection.get(),
                        motion,
                        extend,
                        self.extent(),
                        &occupied,
                    )
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
                Action::CopyValue => {
                    self.copy_value();
                    return glib::Propagation::Stop;
                }
                Action::Fill(dir) => {
                    self.fill(dir);
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

        // --- pointing (doc/sheet-shell.md's formula UX) ---

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
            let text = display::reference_text(&grind_sheet::a1::reference(None, start, end));
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
                .and_then(|app| {
                    app.input_text(self.sheet.get(), self.selection.get().active)
                        .ok()
                })
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
        pub fn copy(&self, cut: bool) {
            let app = self.app.borrow().clone();
            let Some(app) = app else { return };
            let Some((start, end)) = self.clamped_selection() else {
                return;
            };
            let sheet = self.sheet.get();
            self.obj()
                .clipboard()
                .set_text(&self.rect_text(&app, start, end, App::input_text));
            if cut {
                let _ = app.clear_range(sheet, start, end);
                self.obj().queue_draw();
            }
        }

        /// Put the selection's calculated values on the clipboard instead of their formulas
        /// — "Copy Value" (Ctrl+Shift+C), for pasting a result somewhere that should not
        /// follow the document if the source cell later changes.
        pub fn copy_value(&self) {
            let app = self.app.borrow().clone();
            let Some(app) = app else { return };
            let Some((start, end)) = self.clamped_selection() else {
                return;
            };
            self.obj()
                .clipboard()
                .set_text(&self.rect_text(&app, start, end, App::value_text));
        }

        /// Every cell in a rectangle, tab- and newline-separated, by whatever `get` reads
        /// for one — `App::input_text` for `copy`, `App::value_text` for `copy_value`. What
        /// travels is the raw number or the formula in display form, not what the cell
        /// *displays*: pasted back here it reproduces the cells exactly, and pasted into
        /// another spreadsheet `1234.5` is a number where `1,234.50 €` is a guess about that
        /// program's locale — `copy_value` is the one place that guess is exactly the point.
        ///
        /// ponytail: a cell holding a tab or a newline has them replaced with spaces, so the
        /// rectangle survives. The upgrade is quoting, in a codec shared with `sheet paste`.
        fn rect_text(
            &self,
            app: &App,
            start: Pos,
            end: Pos,
            get: impl Fn(&App, usize, Pos) -> grind_sheet::Result<String>,
        ) -> String {
            let sheet = self.sheet.get();
            (start.row..=end.row)
                .map(|row| {
                    (start.col..=end.col)
                        .map(|col| {
                            get(app, sheet, Pos::new(row, col))
                                .unwrap_or_default()
                                .replace(['\t', '\n', '\r'], " ")
                        })
                        .collect::<Vec<_>>()
                        .join("\t")
                })
                .collect::<Vec<_>>()
                .join("\n")
        }

        /// Extend the selection's first row (`Dir::Down`) or first column (`Dir::Right`)
        /// into the rest of it — Ctrl+D / Ctrl+R, the fill-handle drag's keyboard twin.
        /// [`keymap::fill_span`] says which line is the source and which are the targets;
        /// a single selected cell filling into the one after it is the common case. Each
        /// column (for `Down`) or row (for `Right`) keeps its own source,
        /// so a multi-column Fill Down replicates column by column rather than one column's
        /// formula sideways.
        ///
        /// ponytail: one `App::fill` call, and one undo entry, per line — a Fill Down across
        /// five columns is five undo steps rather than one. The upgrade is a multi-source
        /// fill in the core; nothing here needs it until that is a common selection shape.
        pub fn fill(&self, dir: Dir) {
            let app = self.app.borrow().clone();
            let Some(app) = app else { return };
            let (start, end) = self.selection.get().rect();
            let sheet = self.sheet.get();
            match dir {
                Dir::Down => {
                    let Some((src, from, to)) = keymap::fill_span(start.row, end.row) else {
                        return;
                    };
                    for col in start.col..=end.col {
                        let _ = app.fill(
                            sheet,
                            Pos::new(src, col),
                            Pos::new(from, col),
                            Pos::new(to, col),
                            RecalcMode::Document,
                        );
                    }
                }
                Dir::Right => {
                    let Some((src, from, to)) = keymap::fill_span(start.col, end.col) else {
                        return;
                    };
                    for row in start.row..=end.row {
                        let _ = app.fill(
                            sheet,
                            Pos::new(row, src),
                            Pos::new(row, from),
                            Pos::new(row, to),
                            RecalcMode::Document,
                        );
                    }
                }
                Dir::Up | Dir::Left => {}
            }
            self.obj().queue_draw();
        }

        /// The end of a fill-handle drag: the selection replicated into the cells dragged
        /// over, and the selection grown to cover them — the convention every spreadsheet
        /// shares, and this shell's only fill that goes *up* or *left*.
        ///
        /// The source is the selection's edge facing the drag, and each line across that
        /// edge keeps its own source, so dragging a row of formulas down replicates column
        /// by column. Same `App::fill` and same one-undo-entry-per-line ceiling as
        /// [`Self::fill`] above.
        fn commit_fill(&self) {
            self.filling.set(false);
            let Some((dir, line)) = self.fill_to.take() else {
                return;
            };
            let app = self.app.borrow().clone();
            let Some(app) = app else { return };
            let (start, end) = self.selection.get().rect();
            let sheet = self.sheet.get();
            let fill = |source: Pos, from: Pos, to: Pos| {
                let _ = app.fill(sheet, source, from, to, RecalcMode::Document);
            };
            match dir {
                Dir::Down | Dir::Up => {
                    let (src, from, to) = match dir {
                        // `line < start.row` here, so the subtraction cannot wrap.
                        Dir::Up => (start.row, line, start.row - 1),
                        _ => (end.row, end.row + 1, line),
                    };
                    for col in start.col..=end.col {
                        fill(Pos::new(src, col), Pos::new(from, col), Pos::new(to, col));
                    }
                }
                Dir::Right | Dir::Left => {
                    let (src, from, to) = match dir {
                        Dir::Left => (start.col, line, start.col - 1),
                        _ => (end.col, end.col + 1, line),
                    };
                    for row in start.row..=end.row {
                        fill(Pos::new(row, src), Pos::new(row, from), Pos::new(row, to));
                    }
                }
            }
            let (a, b) = keymap::fill_rect(start, end, dir, line);
            self.set_selection(Selection {
                anchor: a,
                active: b,
            });
            self.obj().queue_draw();
        }

        /// Read the clipboard and fill from the selection's top-left corner.
        ///
        /// Asynchronous because the clipboard is: the data may be owned by another process
        /// that has to be asked for it. Every cell goes through the same typing rule a
        /// keystroke does, in one `Action::Batch`, so a paste is one undo step.
        pub fn paste(&self) {
            let (start, _) = self.selection.get().rect();
            self.obj().clipboard().read_text_async(
                gtk::gio::Cancellable::NONE,
                glib::clone!(
                    #[weak(rename_to = grid)]
                    self.obj(),
                    move |result| {
                        let Ok(Some(text)) = result else { return };
                        // Display form back to canonical, cell by cell — the same step
                        // `commit` takes for a single cell. A formula that will not parse
                        // is passed through as typed, which `enter_range` then stores
                        // verbatim rather than losing.
                        let rows: Vec<Vec<String>> = text
                            .lines()
                            .map(|line| {
                                line.split('\t')
                                    .map(|cell| match cell.starts_with('=') {
                                        true => display::from_display(cell)
                                            .unwrap_or_else(|_| cell.to_owned()),
                                        false => cell.to_owned(),
                                    })
                                    .collect()
                            })
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
                                        + rows
                                            .iter()
                                            .map(Vec::len)
                                            .max()
                                            .unwrap_or(1)
                                            .saturating_sub(1)
                                            as u32,
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
        pub fn clear(&self) {
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
            let Some(resize) = self.resize.take() else {
                return;
            };
            let Some(app) = self.app.borrow().clone() else {
                return;
            };
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

        /// A chart's own `svg:x`/`svg:y`/`svg:width`/`svg:height`, converted to on-screen
        /// pixels and placed in widget space — `None` for a length this build cannot parse,
        /// which is a chart this shell simply does not draw rather than a panic (§9's own
        /// tolerance, applied to a shell rather than a reader).
        fn chart_widget_rect(&self, geom: &GridGeom, chart: &grind_sheet::Chart) -> Option<Rect> {
            let zoom = self.zoom.get();
            let px = |length: &str| Some(style::length_mm(length)? * PX_PER_MM * zoom);
            Some(geom.chart_rect(
                px(&chart.x)?,
                px(&chart.y)?,
                px(&chart.width)?,
                px(&chart.height)?,
            ))
        }

        /// What a point hits among this sheet's charts: the resize handle at a chart's own
        /// bottom-right corner beats its body, the same precedence the fill handle gets over
        /// the cell under it — and the *last* chart in `table:shapes`' own order wins a point
        /// two charts both cover, since that is the one drawn on top.
        fn chart_hit(&self, x: f64, y: f64) -> Option<(usize, bool, Rect)> {
            let app = self.app.borrow().clone()?;
            let sheet = self.sheet.get();
            let charts = app.charts(sheet).ok()?;
            let geom = self.geom();
            for (index, chart) in charts.iter().enumerate().rev() {
                let Some(rect) = self.chart_widget_rect(&geom, chart) else {
                    continue;
                };
                let handle = Rect {
                    x: rect.x + rect.w - HANDLE,
                    y: rect.y + rect.h - HANDLE,
                    w: HANDLE * 2.0,
                    h: HANDLE * 2.0,
                };
                if handle.contains(x, y) {
                    return Some((index, true, rect));
                }
                if rect.contains(x, y) {
                    return Some((index, false, rect));
                }
            }
            None
        }

        /// Every chart on this sheet, read fresh and thrown away again like everything else
        /// this widget paints (doc/plan.md rule 1) — a sheet has a handful of charts at most,
        /// so re-resolving each one's live data every frame costs nothing worth caching.
        fn draw_charts(&self, f: &Frame) {
            let Some(app) = self.app.borrow().clone() else {
                return;
            };
            let sheet = self.sheet.get();
            let Ok(charts) = app.charts(sheet) else {
                return;
            };
            let dragging = self.chart_drag.get().map(ChartDrag::index);
            for (index, chart) in charts.iter().enumerate() {
                let at = match (dragging, self.chart_drag_rect.get()) {
                    (Some(i), Some(at)) if i == index => at,
                    _ => match self.chart_widget_rect(&f.geom, chart) {
                        Some(at) => at,
                        None => continue,
                    },
                };
                if at.x + at.w < f.geom.header_w
                    || at.y + at.h < f.geom.header_h
                    || at.x > f.width
                    || at.y > f.height
                {
                    continue;
                }
                let Ok(data) = app.chart_data(sheet, index) else {
                    continue;
                };
                // Every mark's colour resolved the same way the writer resolves one —
                // `grind_sheet::chart::effective_color`, an override if the user picked one
                // else the default cycle — so what is drawn here always matches what gets
                // saved.
                let color = |series: usize, point: Option<usize>| {
                    crate::theme::color(&grind_sheet::effective_color(chart, series, point))
                        .unwrap_or(f.palette.foreground)
                };
                crate::chart::draw(
                    &*self.obj(),
                    f.snapshot,
                    at,
                    chart,
                    &data,
                    &crate::chart::Painter {
                        background: f.palette.background,
                        border: f.palette.lines,
                        foreground: f.palette.foreground,
                        grid: f.palette.lines,
                        color: &color,
                    },
                );
                // The resize handle, the same square the fill handle is — a chart being
                // dragged also gets an accent outline, so the whole shape being moved reads
                // as one thing rather than the drag being invisible until it lands.
                if dragging == Some(index) {
                    outline(f.snapshot, at, f.palette.accent, 2.0);
                }
                f.snapshot.append_color(
                    &f.palette.accent,
                    &rect(at.x + at.w - HANDLE, at.y + at.h - HANDLE, HANDLE, HANDLE),
                );
            }
        }

        /// The end of a chart drag: the widget-space rect becomes ODF lengths and one undo
        /// entry — moving and resizing are otherwise the same call, `App::reshape_chart`.
        fn commit_chart_drag(&self, offset_x: f64, offset_y: f64) {
            let Some(drag) = self.chart_drag.take() else {
                return;
            };
            let Some(rect) = self.chart_drag_rect.take() else {
                return;
            };
            let Some(app) = self.app.borrow().clone() else {
                return;
            };
            // A press that never moved is a click rather than a move — the resize handle has
            // no such thing, since a resize starting and ending on the same pixel is a
            // no-op reshape either way.
            if let ChartDrag::Move {
                index,
                grab_dx,
                grab_dy,
            } = drag
                && offset_x.abs() < CHART_CLICK_THRESHOLD
                && offset_y.abs() < CHART_CLICK_THRESHOLD
            {
                self.obj().queue_draw();
                self.open_chart_color_popover(index, rect.x + grab_dx, rect.y + grab_dy);
                return;
            }
            let geom = self.geom();
            let zoom = self.zoom.get();
            let to_mm = |px: f64| px / zoom / PX_PER_MM;
            let x = style::mm_length(to_mm(rect.x - geom.header_w + geom.scroll_x).max(0.0));
            let y = style::mm_length(to_mm(rect.y - geom.header_h + geom.scroll_y).max(0.0));
            let width = style::mm_length(to_mm(rect.w).max(0.1));
            let height = style::mm_length(to_mm(rect.h).max(0.1));
            let sheet = self.sheet.get();
            if let Err(error) = app.reshape_chart(sheet, drag.index(), &x, &y, &width, &height) {
                self.notice(Notice::Refused(error.to_string()));
            }
            self.obj().queue_draw();
        }

        /// A click that landed on a bar, a pie slice or a line: a palette popover at the
        /// click point, picking from [`crate::formatting::palette_grid`] — the same swatches
        /// a cell's own colour button offers — writes the mark's colour through
        /// `App::set_chart_style` as one undo step. *Automatic* clears the override, back to
        /// [`grind_sheet::series_color`]'s default cycle.
        fn open_chart_color_popover(&self, chart_index: usize, x: f64, y: f64) {
            let Some(app) = self.app.borrow().clone() else {
                return;
            };
            let sheet = self.sheet.get();
            let Ok(charts) = app.charts(sheet) else {
                return;
            };
            let Some(chart) = charts.get(chart_index) else {
                return;
            };
            let Some(rect) = self.chart_widget_rect(&self.geom(), chart) else {
                return;
            };
            let Ok(data) = app.chart_data(sheet, chart_index) else {
                return;
            };
            let Some((series, point)) = crate::chart::mark_at(
                rect,
                chart,
                &data,
                x,
                y,
                &crate::chart::measurer(&*self.obj()),
            ) else {
                // A click on a chart's own background is not a click on anything — editing
                // it is a double-click or the context menu, which is what a user who did not
                // aim at a bar meant.
                return;
            };

            let shown = crate::theme::color(&grind_sheet::effective_color(chart, series, point))
                .unwrap_or(gtk::gdk::RGBA::BLACK);
            let x_axis = chart.x_axis.clone();
            let y_axis = chart.y_axis.clone();
            let series_vec = chart.series.clone();

            let popover = gtk::Popover::new();
            popover.set_parent(&*self.obj());
            popover.set_pointing_to(Some(&gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
            popover.connect_closed(|popover| popover.unparent());

            let widget = self.obj().clone();
            let choices = crate::formatting::palette_grid(&popover, shown, move |picked| {
                let mut series_vec = series_vec.clone();
                let Some(s) = series_vec.get_mut(series) else {
                    return;
                };
                match point {
                    Some(p) => {
                        if s.point_colors.len() <= p {
                            s.point_colors.resize(p + 1, None);
                        }
                        s.point_colors[p] = picked.clone();
                    }
                    None => s.color = picked.clone(),
                }
                if let Err(error) = app.set_chart_style(
                    sheet,
                    chart_index,
                    x_axis.clone(),
                    y_axis.clone(),
                    series_vec,
                ) {
                    widget.imp().notice(Notice::Refused(error.to_string()));
                }
                widget.queue_draw();
            });
            popover.set_child(Some(&choices));
            popover.popup();
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
            let Some(app) = self.app.borrow().clone() else {
                return;
            };
            let sheet = self.sheet.get();
            let Ok((rows, _)) = app.used_extent(sheet) else {
                return;
            };
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
            let width = (width + 2.0 * PAD + FIT_SLACK).max(MIN_TRACK);
            let length = Some(style::mm_length(width / PX_PER_MM));
            if let Err(error) = app.set_col_width(sheet, col..col + 1, length) {
                self.notice(Notice::Refused(error.to_string()));
            }
            self.obj().queue_draw();
        }

        /// Every used column autofit and every explicit row height cleared, in one gesture:
        /// the bulk form of `autofit`/`clear_height` done one boundary at a time. Each
        /// column still gets its own [`App::set_col_width`] call, because the widest text
        /// differs per column, so this is several undo steps rather than one — a coarser
        /// grain than a single drag, but this is not a single drag.
        pub fn autofit_all(&self) {
            let Some(app) = self.app.borrow().clone() else {
                return;
            };
            let sheet = self.sheet.get();
            let Ok((_, cols)) = app.used_extent(sheet) else {
                return;
            };
            for col in 0..cols {
                self.autofit(col);
            }
            if let Ok(heights) = app.row_heights(sheet) {
                for (row, _) in heights {
                    self.clear_height(row);
                }
            }
        }

        // --- the autofilter (§9.4) ---

        /// The sheet's filter, if it has one.
        pub fn filter(&self) -> Option<grind_sheet::Filter> {
            let app = self.app.borrow().clone()?;
            app.filter(self.sheet.get()).ok().flatten()
        }

        /// The dropdown button in one cell, when that cell is a filtered range's heading and
        /// the column is one the filter judges on.
        ///
        /// Two conditions, not one: the range spans whole rows, but a field only exists for
        /// a column inside it, and a button over a column the filter cannot act on would be
        /// a control that does nothing.
        fn filter_button_at(
            geom: &GridGeom,
            filter: &grind_sheet::Filter,
            row: u32,
            col: u32,
        ) -> Option<(u32, Rect)> {
            if row != filter.start.row || col < filter.start.col || col > filter.end.col {
                return None;
            }
            Some((col - filter.start.col, geom.filter_button(row, col)?))
        }

        /// The filter button a point is actually on, if any — what both the press and the
        /// cursor ask, so a button that looks clickable is one and vice versa.
        pub fn filter_button_under(
            &self,
            geom: &GridGeom,
            hit: Hit,
            x: f64,
            y: f64,
        ) -> Option<(u32, Rect)> {
            let Hit::Cell { row, col } = hit else {
                return None;
            };
            let filter = self.filter()?;
            if !filter.buttons {
                return None;
            }
            Self::filter_button_at(geom, &filter, row, col).filter(|(_, b)| b.contains(x, y))
        }

        /// Toggle a filter over the selection — the toolbar's `win.filter`.
        ///
        /// Over a sheet that already has one this clears it, so the button is the on/off
        /// switch its name implies; otherwise the selection becomes the range, with its first
        /// row the heading, which is what a person selecting a table with its titles means.
        pub fn toggle_filter(&self) {
            let Some(app) = self.app.borrow().clone() else {
                return;
            };
            let sheet = self.sheet.get();
            if self.filter().is_some() {
                if let Err(error) = app.set_filter(sheet, None) {
                    self.notice(Notice::Refused(error.to_string()));
                }
                self.obj().queue_draw();
                return;
            }
            let (start, mut end) = self.selection.get().rect();
            // A single cell is a click, not a range: filter the used table around it rather
            // than one cell, which could never hide anything.
            if start == end
                && let Ok((rows, cols)) = app.used_extent(sheet)
            {
                end = Pos::new(rows.saturating_sub(1), cols.saturating_sub(1));
            }
            if end.row <= start.row {
                return self.notice(Notice::Refused(
                    "Select the rows to filter, including their headings".to_owned(),
                ));
            }
            // The name LibreOffice gives an autofilter nobody named; `sheet filter` writes
            // the same one, so a document does not say which shell made it.
            let filter = grind_sheet::Filter::new("__Anonymous_Sheet_DB__0", start, end);
            if let Err(error) = app.set_filter(sheet, Some(filter)) {
                self.notice(Notice::Refused(error.to_string()));
            }
            self.obj().queue_draw();
        }

        /// Open the dropdown for one field, under its button.
        fn open_filter_menu(&self, field: u32, at: Rect) {
            let (Some(app), Some(menu), Some(filter)) = (
                self.app.borrow().clone(),
                self.filter_menu.get(),
                self.filter(),
            ) else {
                return;
            };
            // The whole filtered column, not the visible part: a value scrolled off screen is
            // still one of the column's values.
            let col = filter.column(field);
            let Ok(cells) = app.get_viewport(
                self.sheet.get(),
                filter.start.row..filter.end.row.saturating_add(1),
                col..col.saturating_add(1),
            ) else {
                return;
            };
            let values = crate::filter_ui::field_values(&cells, &filter, field);
            menu.open(at, field, &values, filter.keep.get(&field));
        }

        /// What the dropdown chose, as an undoable change. The whole filter is replaced
        /// because that is the vocabulary `App::set_filter` has — one filter is one value
        /// (`core/src/model.rs`), so a field's condition is edited by reading, changing and
        /// writing it back.
        fn apply_filter(&self, field: u32, chosen: crate::filter_ui::Chosen) {
            let (Some(app), Some(mut filter)) = (self.app.borrow().clone(), self.filter()) else {
                return;
            };
            match chosen {
                // Every value kept is no condition at all, and storing it as one would write
                // a set into the file that says nothing.
                crate::filter_ui::Chosen::Clear => {
                    filter.keep.remove(&field);
                }
                crate::filter_ui::Chosen::Keep(values) => {
                    filter.keep.insert(field, values);
                }
            }
            if let Err(error) = app.set_filter(self.sheet.get(), Some(filter)) {
                self.notice(Notice::Refused(error.to_string()));
            }
            self.obj().queue_draw();
        }

        /// Double-clicking a row boundary: drop the explicit height and let the row fit
        /// itself again, which `autofit`'s doc comment explains.
        fn clear_height(&self, row: u32) {
            let Some(app) = self.app.borrow().clone() else {
                return;
            };
            let sheet = self.sheet.get();
            if let Err(error) = app.set_row_height(sheet, row..row + 1, None) {
                self.notice(Notice::Refused(error.to_string()));
            }
            self.obj().queue_draw();
        }

        /// Clicking a hidden run's marker: show the whole run again, in one undo step.
        fn unhide_cols(&self, from: u32, to: u32) {
            let Some(app) = self.app.borrow().clone() else {
                return;
            };
            let sheet = self.sheet.get();
            if let Err(error) = app.set_col_hidden(sheet, from..to, false) {
                self.notice(Notice::Refused(error.to_string()));
            }
            self.obj().queue_draw();
        }

        /// The row twin of [`Self::unhide_cols`].
        fn unhide_rows(&self, from: u32, to: u32) {
            let Some(app) = self.app.borrow().clone() else {
                return;
            };
            let sheet = self.sheet.get();
            if let Err(error) = app.set_row_hidden(sheet, from..to, false) {
                self.notice(Notice::Refused(error.to_string()));
            }
            self.obj().queue_draw();
        }

        /// Hide a run of columns or rows by hand, from their header's right-click menu.
        fn hide_cols(&self, from: u32, to: u32) {
            let Some(app) = self.app.borrow().clone() else {
                return;
            };
            let sheet = self.sheet.get();
            if let Err(error) = app.set_col_hidden(sheet, from..to, true) {
                self.notice(Notice::Refused(error.to_string()));
            }
            self.obj().queue_draw();
        }

        /// The row twin of [`Self::hide_cols`].
        fn hide_rows(&self, from: u32, to: u32) {
            let Some(app) = self.app.borrow().clone() else {
                return;
            };
            let sheet = self.sheet.get();
            if let Err(error) = app.set_row_hidden(sheet, from..to, true) {
                self.notice(Notice::Refused(error.to_string()));
            }
            self.obj().queue_draw();
        }

        /// Pop the "Hide" menu up at the click point, over the run [`hide_range_for_hit`]
        /// worked out.
        fn open_hide_menu(&self, x: f64, y: f64, is_cols: bool, from: u32, to: u32) {
            let (Some(menu), Some(button)) = (self.hide_menu.get(), self.hide_button.get()) else {
                return;
            };
            self.hide_target.set(Some((is_cols, from, to)));
            button.set_label(match (is_cols, to - from > 1) {
                (true, true) => "Hide Columns",
                (true, false) => "Hide Column",
                (false, true) => "Hide Rows",
                (false, false) => "Hide Row",
            });
            menu.set_pointing_to(Some(&gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
            menu.popup();
        }

        /// The right-click menu over the cells, at the point it was clicked.
        ///
        /// **A cell outside the selection becomes the selection first.** That is what every
        /// grid does and what the menu's own wording assumes: *Clear Contents* on a right
        /// click three cells away from a selection nobody can see any more is the one way a
        /// context menu destroys work.
        fn open_cell_menu(&self, pos: Pos, x: f64, y: f64) {
            let Some(menu) = self.cell_menu.get() else {
                return;
            };
            // An edit in progress is stored first, exactly as a left click elsewhere does —
            // the menu's verbs are about cells, and half-typed text is not in one yet.
            if self.mode.get().is_editing() {
                self.commit(None);
            }
            let (start, end) = self.selection.get().rect();
            let inside = (start.row..=end.row).contains(&pos.row)
                && (start.col..=end.col).contains(&pos.col);
            if !inside {
                self.set_selection(Selection::at(pos));
            }
            menu.set_pointing_to(Some(&gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
            menu.popup();
        }

        /// The right-click menu over a chart, at the point it was clicked.
        fn open_chart_menu(&self, index: usize, x: f64, y: f64) {
            let Some(menu) = self.chart_menu.get() else {
                return;
            };
            self.chart_menu_target.set(Some(index));
            menu.set_pointing_to(Some(&gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
            menu.popup();
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
            // A click on a hidden run's marker unhides it outright — like a double-click on
            // a boundary, it has one unambiguous meaning of its own rather than starting a
            // selection.
            match hit {
                Hit::HiddenCols(from, to) => {
                    self.unhide_cols(from, to);
                    return;
                }
                Hit::HiddenRows(from, to) => {
                    self.unhide_rows(from, to);
                    return;
                }
                _ => {}
            }
            // A chart floats over the cells it sits above, so a press on one grabs the chart
            // rather than starting (or extending) a cell selection underneath it — the same
            // precedence the fill handle gets below, but earlier, since a chart is drawn over
            // everything a filter button or the fill handle could otherwise claim first.
            if let Some((index, resize, rect)) = self.chart_hit(x, y) {
                self.chart_drag.set(Some(match resize {
                    true => ChartDrag::Resize {
                        index,
                        origin_x: rect.x,
                        origin_y: rect.y,
                    },
                    false => ChartDrag::Move {
                        index,
                        grab_dx: x - rect.x,
                        grab_dy: y - rect.y,
                    },
                }));
                self.chart_drag_rect.set(Some(rect));
                self.obj().grab_focus();
                self.obj().queue_draw();
                return;
            }
            // A filter button beats the cell under it, for the same reason the fill handle
            // below does — and before the handle, because the two can overlap on a
            // one-cell selection sitting in the heading row.
            if let Some((field, button)) = self.filter_button_under(&self.geom(), hit, x, y) {
                self.open_filter_menu(field, button);
                return;
            }
            // The fill handle beats the cell under it, for the same reason a boundary beats
            // its header: a 7px target that loses is unreachable.
            let (_, corner) = self.selection.get().rect();
            if self
                .geom()
                .fill_handle(corner.row, corner.col)
                .contains(x, y)
            {
                self.filling.set(true);
                return;
            }
            self.drag.set(Some(hit));
            // A press *on* a boundary is a resize rather than a selection — which is why
            // `Hit` distinguishes the two at all.
            if let Some(size) = self.track_size_of(hit) {
                self.resize.set(Some(Resize { track: hit, size }));
                return;
            }
            let Some(target) = self.selection_for(hit) else {
                return;
            };
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
            // A chart drag only ever repaints its own live rect — nothing is written to the
            // document until the pointer is released (`commit_chart_drag`), which is the
            // whole reason dragging a chart here does not feel like LibreOffice's own.
            if let Some(drag) = self.chart_drag.get() {
                let rect = match drag {
                    ChartDrag::Move {
                        grab_dx, grab_dy, ..
                    } => {
                        let (w, h) = self
                            .chart_drag_rect
                            .get()
                            .map_or((0.0, 0.0), |r| (r.w, r.h));
                        Rect {
                            x: x - grab_dx,
                            y: y - grab_dy,
                            w,
                            h,
                        }
                    }
                    ChartDrag::Resize {
                        origin_x, origin_y, ..
                    } => Rect {
                        x: origin_x,
                        y: origin_y,
                        w: (x - origin_x).max(MIN_CHART),
                        h: (y - origin_y).max(MIN_CHART),
                    },
                };
                self.chart_drag_rect.set(Some(rect));
                self.obj().queue_draw();
                return;
            }
            // A drag from the fill handle grows the *fill*, not the selection: it only
            // outlines where it is pointing until the pointer is released.
            if self.filling.get() {
                let (start, end) = self.selection.get().rect();
                if let Hit::Cell { row, col } = self.geom().hit(x, y) {
                    self.fill_to
                        .set(keymap::fill_target(start, end, Pos::new(row, col)));
                    self.obj().queue_draw();
                }
                return;
            }
            let Some(start) = self.drag.get() else { return };
            // A resize drag moves the boundary itself. The track's *leading* edge does not
            // move, so measuring against it is stable however far the pointer has gone.
            if let Some(resize) = self.resize.get() {
                let geom = self.geom();
                let size = match resize.track {
                    Hit::ColEdge(col) => {
                        x - geom.header_w + geom.scroll_x - geom.cols.offset_of(col)
                    }
                    Hit::RowEdge(row) => {
                        y - geom.header_h + geom.scroll_y - geom.rows.offset_of(row)
                    }
                    _ => return,
                };
                self.resize.set(Some(Resize {
                    size: size.max(MIN_TRACK),
                    ..resize
                }));
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
            let Some(target) = self.selection_for(hit) else {
                return;
            };
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

        /// The a11y floor (`doc/sheet-shell.md`): a custom-drawn grid has no other way to tell
        /// assistive technology the selection moved, so every move speaks the cell's address
        /// and, if it has one, its display text.
        fn announce_active_cell(&self, pos: Pos) {
            let Some(app) = self.app.borrow().clone() else {
                return;
            };
            let address = a1::format(None, pos);
            // The overlays are read here too, and that is `doc/view-modes.md` §4.6 rather
            // than a nicety: a mode whose entire output is colour has to say what it is
            // showing out loud, or it is a feature only some people have.
            let overlays = self.overlays.get();
            let message = match app.get_viewport_with(
                self.sheet.get(),
                pos.row..pos.row + 1,
                pos.col..pos.col + 1,
                overlays,
            ) {
                Ok(viewport) => {
                    let mut message = match viewport.text(pos.row, pos.col) {
                        Some(text) if !text.is_empty() => format!("{address}: {text}"),
                        _ => address,
                    };
                    if let Some(role) = viewport.role(pos.row, pos.col) {
                        message.push_str(", ");
                        message.push_str(role.name());
                    }
                    if let Some(name) = viewport.name_at(pos.row, pos.col) {
                        message.push_str(", named ");
                        message.push_str(name);
                    }
                    message
                }
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
            let zoom = self.zoom.get();
            let m = self.metrics.get();
            let page_w = (f64::from(self.obj().width()) - geom.header_w).max(1.0);
            let page_h = (f64::from(self.obj().height()) - geom.header_h).max(1.0);
            // One default track of context past the target, so a jump never lands with
            // the cursor flush against the edge and whatever comes next hidden.
            let (x, y) = geom.scroll_into_view(
                pos.row,
                pos.col,
                page_w,
                page_h,
                m.col_width * zoom,
                m.row_height * zoom,
            );
            let x = self
                .hadjustment
                .borrow()
                .as_ref()
                .map_or(x, |h| x.min((h.upper() - h.page_size()).max(0.0)));
            let y = self
                .vadjustment
                .borrow()
                .as_ref()
                .map_or(y, |v| y.min((v.upper() - v.page_size()).max(0.0)));
            let moved = (x - geom.scroll_x).abs().max((y - geom.scroll_y).abs());
            if moved == 0.0 {
                return;
            }
            self.stop_glide();
            // A long jump glides — Ctrl+arrow, the name box, "jump to cell" — and a short
            // one is instant. The setting is the user's word on motion, and it is final.
            let animate = self.obj().settings().is_gtk_enable_animations()
                && moved > m.row_height * zoom * GLIDE_AFTER;
            if !animate {
                self.scroll_to(x, y);
                return;
            }
            let (from_x, from_y) = (geom.scroll_x, geom.scroll_y);
            let target = libadwaita::CallbackAnimationTarget::new(glib::clone!(
                #[weak(rename_to = grid)]
                self.obj(),
                move |t| {
                    grid.imp()
                        .scroll_to(from_x + (x - from_x) * t, from_y + (y - from_y) * t);
                }
            ));
            let glide = libadwaita::TimedAnimation::new(&*self.obj(), 0.0, 1.0, GLIDE_MS, target);
            glide.set_easing(libadwaita::Easing::EaseOutCubic);
            glide.play();
            self.glide.replace(Some(glide));
        }

        /// Jump both adjustments. `upper` is grown first when the target lies past it —
        /// [`configure`] recovers the exact upper on the next allocation, and growing it
        /// here is what `configure` itself does with a value past the used extent.
        fn scroll_to(&self, x: f64, y: f64) {
            for (adjustment, value) in [
                (self.hadjustment.borrow(), x),
                (self.vadjustment.borrow(), y),
            ] {
                let Some(adjustment) = adjustment.as_ref() else {
                    continue;
                };
                if value + adjustment.page_size() > adjustment.upper() {
                    adjustment.set_upper(value + adjustment.page_size());
                }
                adjustment.set_value(value);
            }
        }

        /// A newer scroll or a zoom outranks a glide in flight.
        fn stop_glide(&self) {
            if let Some(glide) = self.glide.take() {
                glide.pause();
            }
        }

        /// Change the zoom, keeping the sheet point under `anchor` (widget coordinates)
        /// exactly there — the cell under the pointer stays under the pointer. With no
        /// anchor, the centre of the content area holds still instead.
        pub fn rezoom(&self, zoom: f64, anchor: Option<(f64, f64)>) {
            let zoom = zoom.clamp(*ZOOM_RANGE.start(), *ZOOM_RANGE.end());
            let old = self.zoom.get();
            if zoom == old {
                return;
            }
            self.stop_glide();
            let widget = self.obj();
            let (width, height) = (f64::from(widget.width()), f64::from(widget.height()));
            let geom = self.geom();
            let (ax, ay) = anchor.unwrap_or((
                (geom.header_w + width) / 2.0,
                (geom.header_h + height) / 2.0,
            ));
            // The sheet point under the anchor, in content space, at the old scale. An
            // anchor inside the header band pins the first visible track instead.
            let content_x = geom.scroll_x + (ax - geom.header_w).max(0.0);
            let content_y = geom.scroll_y + (ay - geom.header_h).max(0.0);
            self.zoom.set(zoom);
            // Row heights are measured unzoomed, so they survive; the scrollbars and the
            // editor are both sized in pixels and do not.
            if self.mode.get().is_editing() {
                self.restyle_formula();
            }
            // Every content distance scales by the factor — and so does the header band,
            // which is why the anchor's offset into the content is re-derived against the
            // *new* header size rather than reused.
            let factor = zoom / old;
            let m = self.metrics.get();
            self.scroll_to(
                (content_x * factor - (ax - m.header_w * zoom).max(0.0)).max(0.0),
                (content_y * factor - (ay - m.header_h * zoom).max(0.0)).max(0.0),
            );
            widget.queue_resize();
            widget.queue_draw();
            for hook in self.on_zoom.borrow().iter() {
                hook(zoom);
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
            let active = f
                .geom
                .cell_rect(f.selection.active.row, f.selection.active.col);
            f.snapshot.append_color(
                &f.palette.background,
                &rect(active.x, active.y, active.w, active.h),
            );
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
            for (range, color) in
                crate::theme::reference_colors(&text, crate::theme::is_dark(&f.palette))
            {
                let Ok(reference) = grind_sheet::a1::parse(&text[range.clone()]) else {
                    continue;
                };
                let Ok((sheet, start, end)) = grind_sheet::a1::resolve(&app, &reference) else {
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
            let cell = f
                .geom
                .cell_rect(f.selection.active.row, f.selection.active.col);
            outline(f.snapshot, cell, f.palette.accent, 2.0);
        }

        /// The fill handle on the selection's bottom-right corner, and the rectangle a drag
        /// from it is currently pointing at. Nothing while editing: the corner is where the
        /// in-cell editor is, and a handle there would be a target for a click that means
        /// "put the caret here".
        fn draw_fill(&self, f: &Frame) {
            if self.mode.get().is_editing() {
                return;
            }
            let (start, end) = f.selection.rect();
            if let Some((dir, line)) = self.fill_to.get() {
                let (a, b) = keymap::fill_rect(start, end, dir, line);
                let top_left = f.geom.cell_rect(a.row, a.col);
                let bottom_right = f.geom.cell_rect(b.row, b.col);
                outline(
                    f.snapshot,
                    Rect {
                        x: top_left.x,
                        y: top_left.y,
                        w: bottom_right.x + bottom_right.w - top_left.x,
                        h: bottom_right.y + bottom_right.h - top_left.y,
                    },
                    f.palette.accent,
                    1.0,
                );
            }
            let handle = f.geom.fill_handle(end.row, end.col);
            f.snapshot.append_color(
                &f.palette.accent,
                &rect(handle.x, handle.y, handle.w, handle.h),
            );
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
            // `doc/view-modes.md` §4.5: **in role mode, colour means role, exclusively.**
            // A wash layered over a document that already chose its colours produces cells
            // whose colour has two causes and no way to tell them apart, which is worse than
            // either alone. So the document's own fills are suppressed while the mode is on
            // — and `draw_roles` marks the cells that had one, so nothing is hidden silently.
            if self.overlays.get().roles {
                return;
            }
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
                            border.as_deref().and_then(grind_sheet::style::border_parts)
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
            let editing = self
                .mode
                .get()
                .is_editing()
                .then(|| self.selection.get().active);
            let layout = self.layout();
            // The padding is a distance on screen like everything else here, so it zooms.
            let pad = PAD * self.zoom.get();
            // Role mode puts a one-glyph marker at each cell's leading edge (§4.6), so
            // anything drawn from that edge starts after it. A right-aligned number is
            // untouched: it is at the other end.
            let roles = self.overlays.get().roles;
            let lead = match roles {
                true => pad + ROLE_MARK * self.zoom.get(),
                false => pad,
            };
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
                    // In role mode the colour says what the cell *is*, and the document's
                    // own text colour is suppressed with its fill (§4.5). The font is not:
                    // bold is structure a reader put there, not a second colour channel.
                    let role = roles.then(|| viewport.role(row, col)).flatten();
                    let color = role
                        .and_then(|role| crate::theme::role_color(role, palette))
                        .or_else(|| {
                            style
                                .and_then(|s| s.color.as_deref())
                                .and_then(crate::theme::color)
                        })
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
                        draw_text(
                            f.snapshot,
                            &layout,
                            color,
                            &cell,
                            cell,
                            w,
                            h,
                            (Align::Right, valign),
                            pad,
                        );
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
                        match align {
                            Align::Right => pad,
                            _ => lead,
                        },
                    );
                }
            }
            // The layout is shared and reused, so anything set for one cell has to be unset
            // or the headers inherit it.
            layout.set_attributes(None);
            layout.set_width(-1);
        }

        /// `doc/view-modes.md` Part II in the grid: what every cell *is*, in two channels.
        ///
        /// The colour is `draw_cells`' business — a role is the cell's text colour, which is
        /// the financial-modelling convention this borrows and the reason the document's own
        /// colours are suppressed while the mode is on (§4.5). What is here is everything
        /// that is **not** colour, and §4.6 is why it is not optional: a feature whose entire
        /// output is colour and which ships without a second channel excludes people, and
        /// does not get fixed later.
        ///
        /// * **A one-glyph marker** at each cell's leading edge, one per role, in the role's
        ///   colour but readable without it. `draw_cells` leaves the room for it.
        /// * **A corner triangle** for a role that is also a *diagnostic* — an error, a stale
        ///   value, an unnamed constant. §4.3's distinction, drawn: roles get the colour,
        ///   problems get a mark, and painting both in one channel makes an ordinary model
        ///   look like a wall of warnings.
        /// * **A mark for suppressed styling**, bottom-left, on a cell whose own fill or text
        ///   colour the mode is hiding — so nothing is hidden silently.
        fn draw_roles(&self, f: &Frame) {
            if !self.overlays.get().roles {
                return;
            }
            let Some(viewport) = &f.cells else { return };
            let (geom, palette) = (&f.geom, &f.palette);
            let zoom = self.zoom.get();
            let layout = self.layout();
            layout.set_attributes(
                self.attrs(Some({
                    let list = pango::AttrList::new();
                    list.insert(pango::AttrFloat::new_scale(0.7));
                    list
                }))
                .as_ref(),
            );
            let muted = crate::theme::with_alpha(palette.foreground, 0.4);
            for row in f.rows.clone() {
                for col in f.cols.clone() {
                    let Some(role) = viewport.role(row, col) else {
                        continue;
                    };
                    let Some(color) = crate::theme::role_color(role, palette) else {
                        continue; // an empty cell is the one role drawn as nothing at all.
                    };
                    let cell = geom.cell_rect(row, col);
                    if cell.w <= 0.0 || cell.h <= 0.0 {
                        continue;
                    }
                    layout.set_text(role.marker());
                    let (glyph_w, glyph_h) = layout.pixel_size();
                    f.snapshot.push_clip(&rect(cell.x, cell.y, cell.w, cell.h));
                    f.snapshot.save();
                    f.snapshot.translate(&graphene::Point::new(
                        (cell.x + (ROLE_MARK * zoom - f64::from(glyph_w)).max(0.0) / 2.0) as f32,
                        (cell.y + (cell.h - f64::from(glyph_h)) / 2.0) as f32,
                    ));
                    f.snapshot.append_layout(&layout, &color);
                    f.snapshot.restore();
                    f.snapshot.pop();

                    if role.is_diagnostic() {
                        corner(f.snapshot, cell, color, (5.0 * zoom).min(cell.h / 2.0));
                    }
                    // Suppressed styling: a hairline along the bottom edge, so a reader can
                    // see that this cell looks different outside the mode.
                    let styled = viewport
                        .style(row, col)
                        .is_some_and(|s| s.background.is_some() || s.color.is_some());
                    if styled {
                        f.snapshot.append_color(
                            &muted,
                            &rect(cell.x, cell.y + cell.h - 1.0, cell.w, 1.0),
                        );
                    }
                }
            }
            layout.set_attributes(None);
        }

        /// `doc/view-modes.md` Part I in the grid: where a named expression *lives*, drawn
        /// inside the cell it is bound to.
        ///
        /// The point of it is that a model does not need a label cell beside every constant
        /// — IntelliJ's inlay hints, for a grid. It is a reading of the document and writes
        /// nothing, which is why it can be turned on and off with one key and why the file
        /// is byte-identical either way.
        ///
        /// Three rules, all of them from §3.2 and all of them about *not* becoming noise:
        /// the hint sits at the end opposite the value, the value never yields to it
        /// ([`crate::geom::hint_rect`] elides and then drops), and a hint for a **range** is
        /// drawn once at the first visible cell of it with the rectangle outlined, rather
        /// than forty-nine times.
        fn draw_hints(&self, f: &Frame) {
            if !self.overlays.get().names {
                return;
            }
            let Some(viewport) = &f.cells else { return };
            if viewport.names().is_empty() {
                return;
            }
            let (geom, palette) = (&f.geom, &f.palette);
            let muted = crate::theme::with_alpha(palette.foreground, 0.55);
            let layout = self.layout();
            let pad = PAD * self.zoom.get();
            for anchor in viewport.names() {
                // A range says how far it reaches by being outlined; a single cell needs no
                // outline, since the hint is already inside the only cell it means.
                if anchor.is_range() {
                    let first = geom.cell_rect(anchor.rows.start, anchor.cols.start);
                    let last = geom.cell_rect(anchor.rows.end - 1, anchor.cols.end - 1);
                    outline(
                        f.snapshot,
                        Rect {
                            x: first.x,
                            y: first.y,
                            w: last.x + last.w - first.x,
                            h: last.y + last.h - first.y,
                        },
                        muted,
                        1.0,
                    );
                }
                let Some((row, col)) = geom.hint_cell(
                    (anchor.rows.clone(), anchor.cols.clone()),
                    (f.rows.clone(), f.cols.clone()),
                ) else {
                    continue;
                };
                // With role mode on as well, the leading edge is already spoken for by the
                // role marker, so the hint gets the cell minus that — the same rule the
                // value is drawn under, applied to the other thing sharing the cell.
                let mut cell = geom.cell_rect(row, col);
                if self.overlays.get().roles {
                    let lead = ROLE_MARK * self.zoom.get();
                    cell.x += lead;
                    cell.w -= lead;
                }
                // What the value takes is measured rather than guessed, because that is the
                // whole of "the value never yields".
                let value = viewport.get(row, col);
                let text = viewport.text(row, col).unwrap_or_default();
                layout.set_width(-1);
                layout.set_text(text);
                let value_w = f64::from(layout.pixel_size().0);
                let style = viewport.style(row, col);
                let align = style
                    .and_then(|s| s.align.as_deref())
                    .and_then(aligned)
                    .or_else(|| value.map(alignment))
                    .unwrap_or(Align::Right);
                layout.set_text(&anchor.name);
                let (hint_w, hint_h) = layout.pixel_size();
                let Some(at) = crate::geom::hint_rect(
                    cell,
                    value_w,
                    f64::from(hint_w),
                    align == Align::Right,
                    pad,
                ) else {
                    continue;
                };
                // Elided into whatever room it got — Pango's ellipsis rather than a second
                // opinion about where a word can be cut.
                layout.set_ellipsize(pango::EllipsizeMode::End);
                layout.set_width((at.w * f64::from(pango::SCALE)) as i32);
                f.snapshot.push_clip(&rect(at.x, at.y, at.w, at.h));
                f.snapshot.save();
                f.snapshot.translate(&graphene::Point::new(
                    at.x as f32,
                    (cell.y + (cell.h - f64::from(hint_h)) / 2.0) as f32,
                ));
                f.snapshot.append_layout(&layout, &muted);
                f.snapshot.restore();
                f.snapshot.pop();
            }
            layout.set_ellipsize(pango::EllipsizeMode::None);
            layout.set_width(-1);
        }

        fn draw_headers(&self, f: &Frame) {
            let (snapshot, geom, palette) = (f.snapshot, &f.geom, &f.palette);
            let (width, height, rows, cols) = (f.width, f.height, &f.rows, &f.cols);
            let layout = self.layout();
            // A selected track's label is bold and accented on top of its wash — where the
            // selection is, readable from the edges of the screen without looking for it.
            let plain = self.attrs(None);
            let bold = self.attrs(Some({
                let list = pango::AttrList::new();
                list.insert(pango::AttrInt::new_weight(pango::Weight::Bold));
                list
            }));
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
                let selected = (start.col..=end.col).contains(&col);
                if selected {
                    snapshot.append_color(&wash, &rect(head.x, head.y, head.w, head.h));
                }
                layout.set_attributes(if selected { &bold } else { &plain }.as_ref());
                layout.set_text(&grind_sheet::formula::lex::column_name(col));
                let (w, h) = layout.pixel_size();
                draw_text(
                    snapshot,
                    &layout,
                    match selected {
                        true => palette.accent,
                        false => palette.header_text,
                    },
                    &head,
                    head,
                    w,
                    h,
                    (Align::Center, VAlign::Middle),
                    PAD,
                );
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
                let selected = (start.row..=end.row).contains(&row);
                if selected {
                    snapshot.append_color(&wash, &rect(head.x, head.y, head.w, head.h));
                }
                layout.set_attributes(if selected { &bold } else { &plain }.as_ref());
                layout.set_text(&(row + 1).to_string());
                let (w, h) = layout.pixel_size();
                draw_text(
                    snapshot,
                    &layout,
                    match selected {
                        true => palette.accent,
                        false => palette.header_text,
                    },
                    &head,
                    head,
                    w,
                    h,
                    (Align::Center, VAlign::Middle),
                    PAD,
                );
            }
            snapshot.pop();

            layout.set_attributes(None);
            let line = with_alpha(palette.lines, 1.0);
            snapshot.append_color(&line, &rect(0.0, geom.header_h - 1.0, width, 1.0));
            snapshot.append_color(&line, &rect(geom.header_w - 1.0, 0.0, 1.0, height));
        }

        /// A run of columns or rows hidden by hand (§5.4): a thin accent bar standing where
        /// the run collapsed to nothing, between the two headers that still show either
        /// side of it — the one visible trace of the run, and clicking it unhides the whole
        /// thing (`Grid::press`).
        ///
        /// Only *manually* hidden tracks get a mark here — a row the filter hides has its
        /// own dropdown already saying why (`draw_filter_buttons`'s doc comment), and giving
        /// it a second, different-looking control would be two ways to ask the same
        /// question.
        fn draw_hidden_markers(&self, f: &Frame) {
            let Some(app) = self.app.borrow().clone() else {
                return;
            };
            let sheet = self.sheet.get();
            let cols: std::collections::BTreeSet<u32> = app
                .hidden_cols(sheet)
                .unwrap_or_default()
                .into_iter()
                .collect();
            let rows: std::collections::BTreeSet<u32> = app
                .manually_hidden_rows(sheet)
                .unwrap_or_default()
                .into_iter()
                .collect();

            // One mark per run, at its first hidden index — every other index in the run
            // shares the same collapsed offset, so drawing all of them would stack the same
            // bar on itself.
            for col in f.cols.clone() {
                if cols.contains(&col) && (col == 0 || !cols.contains(&(col - 1))) {
                    let bar = f.geom.hidden_col_marker(col);
                    f.snapshot
                        .append_color(&f.palette.accent, &rect(bar.x, bar.y, bar.w, bar.h));
                }
            }
            for row in f.rows.clone() {
                if rows.contains(&row) && (row == 0 || !rows.contains(&(row - 1))) {
                    let bar = f.geom.hidden_row_marker(row);
                    f.snapshot
                        .append_color(&f.palette.accent, &rect(bar.x, bar.y, bar.w, bar.h));
                }
            }
        }

        /// The autofilter's dropdown buttons, one per field, in the range's heading row
        /// (§9.4).
        ///
        /// The button's face is always the sheet's own background, never the cell's. A
        /// heading is very often given a strong fill by the document — the sample's is solid
        /// blue — and a button tinted to match it stops looking like a control at all. A
        /// constant light chip with a border reads as one on any heading.
        ///
        /// Which leaves the *state* to the glyph rather than the fill: a filtered field gets
        /// an accent chevron over an accent underline, an unfiltered one a plain chevron.
        /// State has to survive being drawn on top of an accent-coloured heading, so it
        /// cannot itself be "fill the chip with the accent" — that is the one combination
        /// that disappears. "Which column is why rows are missing" is the question a filtered
        /// sheet raises, and a gap in the row numbers alone cannot answer it: rows somebody
        /// hid by hand look identical.
        ///
        /// `table:display-filter-buttons="false"` is honoured: the document asked for no
        /// buttons, and the toolbar's Filter action still reaches the thing.
        fn draw_filter_buttons(&self, f: &Frame) {
            let Some(filter) = &f.filter else { return };
            if !filter.buttons {
                return;
            }
            let layout = self.layout();
            layout.set_attributes(self.attrs(None).as_ref());
            layout.set_width(-1);
            layout.set_text(CHEVRON);
            let (text_w, text_h) = layout.pixel_size();

            for col in f.cols.clone() {
                let Some((field, button)) =
                    Self::filter_button_at(&f.geom, filter, filter.start.row, col)
                else {
                    continue;
                };
                let on = filter.keep.contains_key(&field);
                let rounded =
                    gsk::RoundedRect::from_rect(rect(button.x, button.y, button.w, button.h), 3.0);
                f.snapshot.push_rounded_clip(&rounded);
                f.snapshot.append_color(
                    &f.palette.background,
                    &rect(button.x, button.y, button.w, button.h),
                );
                // The filtered field's underline, inside the chip so the border keeps its
                // edge — the mark that survives any heading colour behind it.
                if on {
                    f.snapshot.append_color(
                        &f.palette.accent,
                        &rect(
                            button.x,
                            button.y + button.h - UNDERLINE,
                            button.w,
                            UNDERLINE,
                        ),
                    );
                }
                f.snapshot.pop();
                let edge = match on {
                    true => f.palette.accent,
                    false => f.palette.lines,
                };
                f.snapshot.append_border(&rounded, &[1.0; 4], &[edge; 4]);
                draw_text(
                    f.snapshot,
                    &layout,
                    match on {
                        true => f.palette.accent,
                        false => f.palette.foreground,
                    },
                    &button,
                    button,
                    text_w,
                    // The glyph sits clear of the underline rather than on it.
                    text_h,
                    (Align::Center, VAlign::Middle),
                    0.0,
                );
            }
            layout.set_attributes(None);
        }

        /// While a boundary is being dragged, the size it is choosing — as the document
        /// length that will be stored, beside the boundary, where the eye already is.
        fn draw_resize_hint(&self, f: &Frame) {
            let Some(resize) = self.resize.get() else {
                return;
            };
            // The same conversion `commit_resize` stores, shown in centimetres because
            // that is the unit the rest of the document's lengths speak.
            let mm = resize.size / self.zoom.get() / PX_PER_MM;
            let text = format!("{:.2} cm", mm / 10.0);
            let layout = self.layout();
            layout.set_attributes(self.attrs(None).as_ref());
            layout.set_width(-1);
            layout.set_text(&text);
            let (w, h) = layout.pixel_size();
            let (w, h) = (f64::from(w), f64::from(h));
            // Just past the moving edge, clear of the header band — and clamped inside
            // the view, so dragging a track wider than the window keeps the number on it.
            let (x, y) = match resize.track {
                Hit::ColEdge(col) => (
                    f.geom.header_w + f.geom.cols.offset_of(col) - f.geom.scroll_x
                        + resize.size
                        + 8.0,
                    f.geom.header_h + 8.0,
                ),
                Hit::RowEdge(row) => (
                    f.geom.header_w + 8.0,
                    f.geom.header_h + f.geom.rows.offset_of(row) - f.geom.scroll_y
                        + resize.size
                        + 8.0,
                ),
                _ => return,
            };
            let pad = 6.0;
            let bubble = Rect {
                x: x.min(f.width - w - 2.0 * pad - 4.0).max(f.geom.header_w),
                y: y.min(f.height - h - 2.0 * pad - 4.0).max(f.geom.header_h),
                w: w + 2.0 * pad,
                h: h + 2.0 * pad,
            };
            let rounded =
                gsk::RoundedRect::from_rect(rect(bubble.x, bubble.y, bubble.w, bubble.h), 6.0);
            f.snapshot.push_rounded_clip(&rounded);
            f.snapshot.append_color(
                &f.palette.header,
                &rect(bubble.x, bubble.y, bubble.w, bubble.h),
            );
            f.snapshot.pop();
            f.snapshot
                .append_border(&rounded, &[1.0; 4], &[f.palette.lines; 4]);
            f.snapshot.save();
            f.snapshot.translate(&graphene::Point::new(
                (bubble.x + pad) as f32,
                (bubble.y + pad) as f32,
            ));
            f.snapshot.append_layout(&layout, &f.palette.foreground);
            f.snapshot.restore();
            layout.set_attributes(None);
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
    fn font(style: &grind_sheet::style::CellStyle) -> Option<pango::AttrList> {
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
            attrs.insert(pango::AttrSize::new(
                (points * f64::from(pango::SCALE)) as i32,
            ));
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
        text.get(..byte)
            .map_or(-1, |head| head.chars().count() as i32)
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

    /// An outlined rectangle with softened corners — the active cell's cursor and the
    /// reference outlines. The radius is small enough that the rectangle still reads as
    /// exactly the cells it covers; the grid lines and the selection wash stay square.
    /// A filled triangle in a cell's top-right corner — §4.3's diagnostic mark, the same
    /// shape a spreadsheet has used for "there is something here" since forever.
    fn corner(snapshot: &gtk::Snapshot, cell: Rect, color: gtk::gdk::RGBA, size: f64) {
        let builder = gsk::PathBuilder::new();
        let (x, y) = ((cell.x + cell.w) as f32, cell.y as f32);
        builder.move_to(x - size as f32, y);
        builder.line_to(x, y);
        builder.line_to(x, y + size as f32);
        builder.close();
        snapshot.append_fill(&builder.to_path(), gsk::FillRule::Winding, &color);
    }

    fn outline(snapshot: &gtk::Snapshot, r: Rect, color: gtk::gdk::RGBA, t: f64) {
        let bounds =
            gsk::RoundedRect::from_rect(rect(r.x - 1.0, r.y - 1.0, r.w + 2.0, r.h + 2.0), 3.0);
        snapshot.append_border(&bounds, &[t as f32; 4], &[color; 4]);
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
