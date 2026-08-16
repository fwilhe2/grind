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

use std::cell::{Cell, RefCell};
use std::sync::Arc;

use libadwaita::gtk;
use libadwaita::prelude::*;

use gtk::glib;
use libadwaita::subclass::prelude::ObjectSubclassIsExt;
use sheet_core::App;

use crate::geom::{ColWidths, GridGeom, MAX_COLS, MAX_ROWS};

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
        self.queue_draw();
    }
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
    use sheet_core::CellValue;
    use sheet_core::formula::value::FormulaError;

    use crate::geom::Rect;
    use crate::theme::{Palette, with_alpha};

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

        fn constructed(&self) {
            self.parent_constructed();
            self.obj().set_focusable(true);
        }
    }

    impl ScrollableImpl for Grid {}

    impl WidgetImpl for Grid {
        /// A scrollable asks for nothing and takes what it is given; the scrolled window
        /// decides the size and this decides what fits in it.
        fn measure(&self, _orientation: gtk::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
            (0, 0, -1, -1)
        }

        fn size_allocate(&self, width: i32, height: i32, _baseline: i32) {
            self.configure_adjustments(f64::from(width), f64::from(height));
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
            };

            snapshot.append_color(&frame.palette.background, &rect(0.0, 0.0, width, height));

            snapshot.push_clip(&rect(
                geom.header_w,
                geom.header_h,
                width - geom.header_w,
                height - geom.header_h,
            ));
            self.draw_lines(&frame);
            self.draw_cells(&frame);
            snapshot.pop();

            self.draw_headers(&frame);
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
                move |_| grid.queue_draw()
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

            let layout = self.layout();
            for row in rows.clone() {
                for col in fetch.clone() {
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

            snapshot.push_clip(&rect(geom.header_w, 0.0, width, geom.header_h));
            for col in cols.clone() {
                let cell = geom.cell_rect(0, col);
                let head = Rect {
                    y: 0.0,
                    h: geom.header_h,
                    ..cell
                };
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
