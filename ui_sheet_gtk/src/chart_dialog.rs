// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The chart dialog — **one dialog, both jobs**: inserting a chart and editing one, built around
//! a live preview of exactly what will be written (`doc/chart-format.md`, The shell).
//!
//! The preview is the point. It is drawn by the same [`crate::chart::draw`] the grid draws a
//! chart with, from [`App::preview_chart`] — the core's answer to "what would this spec make",
//! which is [`App::add_chart`] or [`App::edit_chart`] without the write. So the picture in the
//! dialog and the chart that lands on the sheet cannot disagree, and nothing is written until
//! Insert, as one undo step.
//!
//! **What the chart is of is asked in the table's own terms**, not in range strings: which
//! block, whether the series run down the columns or along the rows, and whether the first row
//! and the first column name things. Those are [`grind_sheet::ChartShape`]'s three answers, and
//! [`App::suggest_chart`] turns them into a chart — the same guess `chart-add --from` makes, so
//! a table with its months across the top charts them along the x axis without being told. The
//! ranges themselves — the CLI's vocabulary — are still there, folded away under *Ranges*, for
//! anybody who wants to point a series somewhere the table's shape does not.
//!
//! **The widgets are the state.** Every change re-reads them into a [`ChartSpec`], asks the
//! core for a preview, and redraws; nothing here keeps a second copy of the chart to drift out
//! of step with what is on screen. Two flags are the exception, and each is a fact about the
//! *person*: whether they typed a title, and whether they picked a legend — until they do, a new
//! reading of the table may give the chart a title and a legend of its own, and once they have,
//! it does not overwrite what they chose.
//!
//! Deliberately not built: a gallery of chart types that changes the document as the pointer
//! passes over it, a contextual toolbar, or a floating button offering analyses. The preview
//! lives inside a modal dialog, which is ordinary GNOME practice, and a type is picked by
//! pressing one of three buttons.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use libadwaita as adw;
use libadwaita::gtk;
use libadwaita::prelude::*;
use libadwaita::subclass::prelude::*;

use gtk::glib;

use grind_sheet::{App, Chart, ChartData, ChartKind, ChartLegend, ChartShape, ChartSpec, Pos, a1};

use crate::geom::Rect;
use crate::grid::Grid;

/// The three kinds, in the order their buttons stand.
const KINDS: [(ChartKind, &str); 3] = [
    (ChartKind::Bar, "Bar"),
    (ChartKind::Line, "Line"),
    (ChartKind::Pie, "Pie"),
];

/// The legend's positions as the dropdown offers them, "None" first. `End` is the right-hand
/// side and `Start` the left: ODF's words are about the writing direction, and this window
/// lays a chart out left to right (`doc/text-layout.md` puts RTL out of scope).
const LEGENDS: [(Option<ChartLegend>, &str); 5] = [
    (None, "None"),
    (Some(ChartLegend::End), "Right"),
    (Some(ChartLegend::Bottom), "Bottom"),
    (Some(ChartLegend::Top), "Top"),
    (Some(ChartLegend::Start), "Left"),
];

/// Where a new chart is drawn, and how big: a size that reads at a glance and leaves the
/// sheet around it visible. A drag changes both afterwards.
const NEW_WIDTH: &str = "12cm";
const NEW_HEIGHT: &str = "7.5cm";

/// Open the dialog over `window`: `None` inserts a chart of the grid's selection, `Some(index)`
/// edits that chart on the grid's sheet.
pub fn present(
    window: &impl IsA<gtk::Widget>,
    app: &Arc<App>,
    grid: &Grid,
    editing: Option<usize>,
) {
    let sheet = grid.sheet();
    let existing = match editing {
        Some(index) => match app.charts(sheet).ok().and_then(|c| c.get(index).cloned()) {
            Some(chart) => Some(chart),
            None => return,
        },
        None => None,
    };
    let this = Dialog::build(app, grid, editing);
    match &existing {
        Some(chart) => this.open_on(chart),
        None => {
            let (start, end) = grid.selection().rect();
            this.open_on_block(start, end);
        }
    }
    // The dialog holds itself for as long as it is open, and lets go when it closes — every
    // signal handler below holds it weakly, so this is the one strong reference.
    let keep = RefCell::new(Some(this.clone()));
    this.dialog.connect_closed(move |_| {
        keep.borrow_mut().take();
    });
    this.dialog.present(Some(window));
}

/// One axis' worth of rows: its title, and a switch each for its labels and its gridlines.
struct AxisRows {
    expander: adw::ExpanderRow,
    title: adw::EntryRow,
    labels: adw::SwitchRow,
    gridlines: adw::SwitchRow,
}

impl AxisRows {
    fn new(name: &str) -> Self {
        let expander = adw::ExpanderRow::builder().title(name).build();
        let title = adw::EntryRow::builder().title("Title").build();
        let labels = adw::SwitchRow::builder()
            .title("Labels")
            .subtitle("Name each value along this axis")
            .build();
        let gridlines = adw::SwitchRow::builder().title("Gridlines").build();
        expander.add_row(&title);
        expander.add_row(&labels);
        expander.add_row(&gridlines);
        AxisRows {
            expander,
            title,
            labels,
            gridlines,
        }
    }

    fn read(&self) -> grind_sheet::ChartAxis {
        let title = self.title.text();
        grind_sheet::ChartAxis {
            label: (!title.trim().is_empty()).then(|| title.trim().to_owned()),
            tick_labels: self.labels.is_active(),
            gridlines: self.gridlines.is_active(),
        }
    }

    fn show(&self, axis: &grind_sheet::ChartAxis) {
        self.title
            .set_text(axis.label.as_deref().unwrap_or_default());
        self.labels.set_active(axis.tick_labels);
        self.gridlines.set_active(axis.gridlines);
        // An axis somebody has already said something about opens showing it.
        self.expander
            .set_expanded(axis.label.is_some() || axis.gridlines || !axis.tick_labels);
        self.expander.set_subtitle(&axis_summary(axis));
    }
}

struct Dialog {
    app: Arc<App>,
    grid: Grid,
    sheet: usize,
    editing: Option<usize>,
    dialog: adw::Dialog,
    title_widget: adw::WindowTitle,
    toasts: adw::ToastOverlay,
    preview: Preview,
    kinds: Vec<gtk::ToggleButton>,
    range: adw::EntryRow,
    by_rows: adw::ComboRow,
    header_row: adw::SwitchRow,
    label_column: adw::SwitchRow,
    ranges: adw::ExpanderRow,
    categories: adw::EntryRow,
    series: RefCell<Vec<adw::EntryRow>>,
    add_series: adw::ActionRow,
    title: adw::EntryRow,
    legend: adw::ComboRow,
    clockwise: adw::SwitchRow,
    axes: adw::PreferencesGroup,
    x: AxisRows,
    y: AxisRows,
    insert: gtk::Button,
    /// Raised while this code is writing into its own widgets, so their signals can tell a
    /// refresh from a person — the same latch `formatting::Strip` has.
    updating: Cell<bool>,
    /// Whether the person typed the title or picked the legend themselves; until they do, both
    /// follow the chart (a single series' name, a legend when there is more than one thing).
    title_touched: Cell<bool>,
    legend_touched: Cell<bool>,
    /// The block the ranges were last read from, when they were read from one — where a new
    /// chart is placed beside.
    block: Cell<Option<(Pos, Pos)>>,
}

impl Dialog {
    fn build(app: &Arc<App>, grid: &Grid, editing: Option<usize>) -> Rc<Self> {
        let preview = Preview::new();
        let card = gtk::Box::new(gtk::Orientation::Vertical, 0);
        card.add_css_class("card");
        card.set_overflow(gtk::Overflow::Hidden);
        card.set_margin_top(12);
        card.set_margin_start(12);
        card.set_margin_end(12);
        card.set_margin_bottom(6);
        card.append(&preview);

        // The type, as three buttons that press each other out — a glyph and a word each,
        // since Adwaita's icon theme has no chart icons and a word is what a person reads.
        let type_row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        type_row.add_css_class("linked");
        type_row.set_homogeneous(true);
        type_row.set_halign(gtk::Align::Center);
        type_row.set_margin_top(6);
        type_row.set_margin_bottom(6);
        let kinds: Vec<gtk::ToggleButton> = KINDS
            .iter()
            .map(|(kind, name)| {
                let content = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                content.set_halign(gtk::Align::Center);
                content.append(&glyph(*kind));
                content.append(&gtk::Label::new(Some(name)));
                let button = gtk::ToggleButton::builder()
                    .child(&content)
                    .tooltip_text(format!("{name} Chart"))
                    .build();
                button.set_width_request(96);
                type_row.append(&button);
                button
            })
            .collect();
        for button in kinds.iter().skip(1) {
            button.set_group(Some(&kinds[0]));
        }

        let range = adw::EntryRow::builder().title("Range").build();
        let by_rows = adw::ComboRow::builder()
            .title("Series in")
            .model(&gtk::StringList::new(&["Columns", "Rows"]))
            .build();
        let header_row = adw::SwitchRow::builder()
            .title("First row is labels")
            .build();
        let label_column = adw::SwitchRow::builder()
            .title("First column is labels")
            .build();
        let ranges = adw::ExpanderRow::builder()
            .title("Ranges")
            .subtitle("Point a series anywhere")
            .build();
        let categories = adw::EntryRow::builder().title("Categories").build();
        ranges.add_row(&categories);
        let add_series = adw::ActionRow::builder()
            .title("Add Series")
            .activatable(true)
            .build();
        add_series.add_prefix(&gtk::Image::from_icon_name("list-add-symbolic"));
        ranges.add_row(&add_series);
        let data = adw::PreferencesGroup::builder().title("Data").build();
        for row in [
            range.upcast_ref::<gtk::Widget>(),
            by_rows.upcast_ref(),
            header_row.upcast_ref(),
            label_column.upcast_ref(),
            ranges.upcast_ref(),
        ] {
            data.add(row);
        }

        let title = adw::EntryRow::builder().title("Title").build();
        let legend = adw::ComboRow::builder()
            .title("Legend")
            .model(&gtk::StringList::new(
                &LEGENDS.iter().map(|(_, name)| *name).collect::<Vec<_>>(),
            ))
            .build();
        let clockwise = adw::SwitchRow::builder()
            .title("Clockwise")
            .subtitle("Slices run from twelve o'clock the way a clock does")
            .active(true)
            .build();
        let labels = adw::PreferencesGroup::builder().title("Labels").build();
        labels.add(&title);
        labels.add(&legend);
        labels.add(&clockwise);

        let x = AxisRows::new("Horizontal Axis");
        let y = AxisRows::new("Vertical Axis");
        let axes = adw::PreferencesGroup::builder().title("Axes").build();
        axes.add(&x.expander);
        axes.add(&y.expander);

        let page = adw::PreferencesPage::new();
        page.add(&data);
        page.add(&labels);
        page.add(&axes);

        let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
        body.append(&card);
        body.append(&type_row);
        page.set_vexpand(true);
        body.append(&page);

        let toasts = adw::ToastOverlay::new();
        toasts.set_child(Some(&body));

        let cancel = gtk::Button::with_label("Cancel");
        let insert = gtk::Button::with_label(match editing {
            Some(_) => "Apply",
            None => "Insert",
        });
        insert.add_css_class("suggested-action");
        let title_widget = adw::WindowTitle::new(
            match editing {
                Some(_) => "Edit Chart",
                None => "Insert Chart",
            },
            "",
        );
        let header = adw::HeaderBar::builder()
            .show_start_title_buttons(false)
            .show_end_title_buttons(false)
            .title_widget(&title_widget)
            .build();
        header.pack_start(&cancel);
        header.pack_end(&insert);

        let view = adw::ToolbarView::builder().content(&toasts).build();
        view.add_top_bar(&header);
        let dialog = adw::Dialog::builder()
            .title(title_widget.title())
            .content_width(560)
            .content_height(760)
            .child(&view)
            .build();
        dialog.set_default_widget(Some(&insert));
        dialog.set_focus(Some(&kinds[0]));

        let this = Rc::new(Dialog {
            app: app.clone(),
            grid: grid.clone(),
            sheet: grid.sheet(),
            editing,
            dialog,
            title_widget,
            toasts,
            preview,
            kinds,
            range,
            by_rows,
            header_row,
            label_column,
            ranges,
            categories,
            series: RefCell::new(Vec::new()),
            add_series,
            title,
            legend,
            clockwise,
            axes,
            x,
            y,
            insert,
            updating: Cell::new(false),
            title_touched: Cell::new(false),
            legend_touched: Cell::new(false),
            block: Cell::new(None),
        });
        this.wire(&cancel);
        this
    }

    /// Every signal, each holding the dialog weakly (see [`present`]).
    fn wire(self: &Rc<Self>, cancel: &gtk::Button) {
        let weak = Rc::downgrade(self);
        let on = move |f: fn(&Rc<Dialog>)| {
            let weak = weak.clone();
            move || {
                if let Some(this) = weak.upgrade()
                    && !this.updating.get()
                {
                    f(&this);
                }
            }
        };

        for button in &self.kinds {
            let changed = on(|this| this.refresh());
            button.connect_toggled(move |button| {
                if button.is_active() {
                    changed();
                }
            });
        }
        let changed = on(|this| this.reread(None));
        self.range.connect_changed(move |_| changed());
        for shape in [&self.header_row, &self.label_column] {
            let changed = on(|this| this.reread(Some(this.shape())));
            shape.connect_active_notify(move |_| changed());
        }
        let changed = on(|this| this.reread(Some(this.shape())));
        self.by_rows.connect_selected_notify(move |_| changed());
        let changed = on(|this| this.refresh());
        self.categories.connect_changed(move |_| changed());
        let add = on(|this| {
            this.push_series("");
            this.refresh();
        });
        self.add_series.connect_activated(move |_| add());

        let typed = on(|this| {
            this.title_touched.set(true);
            this.refresh();
        });
        self.title.connect_changed(move |_| typed());
        let picked = on(|this| {
            this.legend_touched.set(true);
            this.refresh();
        });
        self.legend.connect_selected_notify(move |_| picked());
        let changed = on(|this| this.refresh());
        self.clockwise.connect_active_notify(move |_| changed());
        for axis in [&self.x, &self.y] {
            let changed = on(|this| this.refresh());
            axis.title.connect_changed(move |_| changed());
            let changed = on(|this| this.refresh());
            axis.labels.connect_active_notify(move |_| changed());
            let changed = on(|this| this.refresh());
            axis.gridlines.connect_active_notify(move |_| changed());
        }

        let commit = on(|this| this.commit());
        self.insert.connect_clicked(move |_| commit());
        // Enter in any text row inserts, the way the default button of a dialog does.
        for row in [
            &self.range,
            &self.categories,
            &self.title,
            &self.x.title,
            &self.y.title,
        ] {
            let commit = on(|this| this.commit());
            row.connect_entry_activated(move |_| commit());
        }
        let dialog = self.dialog.downgrade();
        cancel.connect_clicked(move |_| {
            if let Some(dialog) = dialog.upgrade() {
                dialog.close();
            }
        });
    }

    // --- opening ---

    /// Insert: the block the selection is in, read the way `chart-add --from` reads one.
    fn open_on_block(self: &Rc<Self>, start: Pos, end: Pos) {
        let Ok(guessed) = self.app.suggest_chart(self.sheet, start, end, None) else {
            return;
        };
        self.updating.set(true);
        self.range.set_text(&block_text(guessed.start, guessed.end));
        self.show_kind(guessed.spec.kind);
        self.show_shape(guessed.shape);
        self.show_ranges(&guessed.spec);
        self.title
            .set_text(guessed.spec.title.as_deref().unwrap_or_default());
        self.show_legend(guessed.spec.legend);
        self.clockwise.set_active(guessed.spec.clockwise);
        self.x.show(&guessed.spec.x_axis);
        self.y.show(&guessed.spec.y_axis);
        self.updating.set(false);
        self.block.set(Some((guessed.start, guessed.end)));
        self.refresh();
    }

    /// Edit: the chart as it is. Its ranges are shown as they are, and the table they span is
    /// shown as the range, read afresh — but nothing is re-guessed until the person changes the
    /// range or how it is read, so opening and applying a chart leaves it exactly as it was.
    fn open_on(self: &Rc<Self>, chart: &Chart) {
        let spec = ChartSpec::of(chart);
        let name = self.app.sheet_name(self.sheet).unwrap_or_default();
        let block = spanned(&self.app, self.sheet, &spec);
        self.updating.set(true);
        if let Some((start, end)) = block
            && let Ok(guessed) = self.app.suggest_chart(self.sheet, start, end, None)
        {
            self.range.set_text(&block_text(start, end));
            self.show_shape(guessed.shape);
        }
        self.show_kind(spec.kind);
        let local = ChartSpec {
            categories: spec.categories.as_deref().map(|r| plain_range(&name, r)),
            series: spec
                .series
                .iter()
                .map(|(values, label)| {
                    (
                        plain_range(&name, values),
                        label.as_deref().map(|l| plain_range(&name, l)),
                    )
                })
                .collect(),
            ..spec.clone()
        };
        self.show_ranges(&local);
        self.title
            .set_text(spec.title.as_deref().unwrap_or_default());
        self.show_legend(spec.legend);
        self.clockwise.set_active(spec.clockwise);
        self.x.show(&spec.x_axis);
        self.y.show(&spec.y_axis);
        self.updating.set(false);
        // A chart that exists has already said what its title and legend are.
        self.title_touched.set(true);
        self.legend_touched.set(true);
        self.block.set(block);
        self.refresh();
    }

    // --- reading the widgets ---

    fn kind(&self) -> ChartKind {
        KINDS
            .iter()
            .zip(&self.kinds)
            .find(|(_, button)| button.is_active())
            .map_or(ChartKind::Bar, |((kind, _), _)| *kind)
    }

    fn shape(&self) -> ChartShape {
        ChartShape {
            by_rows: self.by_rows.selected() == 1,
            header_row: self.header_row.is_active(),
            label_column: self.label_column.is_active(),
        }
    }

    /// The chart the widgets describe.
    fn read(&self) -> ChartSpec {
        let categories = self.categories.text();
        ChartSpec {
            kind: self.kind(),
            categories: (!categories.trim().is_empty()).then(|| categories.trim().to_owned()),
            series: self
                .series
                .borrow()
                .iter()
                .filter_map(|row| parse_series(&row.text()))
                .collect(),
            x_axis: self.x.read(),
            y_axis: self.y.read(),
            clockwise: self.clockwise.is_active(),
            title: Some(self.title.text().trim().to_owned()).filter(|t| !t.is_empty()),
            legend: LEGENDS
                .get(self.legend.selected() as usize)
                .and_then(|(legend, _)| *legend),
        }
    }

    // --- writing the widgets ---

    fn show_kind(&self, kind: ChartKind) {
        if let Some(index) = KINDS.iter().position(|(k, _)| *k == kind) {
            self.kinds[index].set_active(true);
        }
    }

    fn show_shape(&self, shape: ChartShape) {
        self.by_rows.set_selected(u32::from(shape.by_rows));
        self.header_row.set_active(shape.header_row);
        self.label_column.set_active(shape.label_column);
    }

    fn show_legend(&self, legend: Option<ChartLegend>) {
        let index = LEGENDS.iter().position(|(l, _)| *l == legend).unwrap_or(0);
        self.legend.set_selected(index as u32);
    }

    /// The categories and the series rows, rebuilt to say what `spec` says.
    fn show_ranges(self: &Rc<Self>, spec: &ChartSpec) {
        self.categories
            .set_text(spec.categories.as_deref().unwrap_or_default());
        for row in self.series.borrow_mut().drain(..) {
            self.ranges.remove(&row);
        }
        for series in &spec.series {
            self.push_series(&series_text(series));
        }
    }

    /// One more series row — `RANGE=LABEL`, the vocabulary `chart-add --series` takes — with a
    /// button that takes it away again.
    fn push_series(self: &Rc<Self>, text: &str) {
        let index = self.series.borrow().len();
        let row = adw::EntryRow::builder()
            .title(format!("Series {}", index + 1))
            .text(text)
            .build();
        let remove = gtk::Button::from_icon_name("user-trash-symbolic");
        remove.set_tooltip_text(Some("Remove Series"));
        remove.set_valign(gtk::Align::Center);
        remove.add_css_class("flat");
        let weak = Rc::downgrade(self);
        let row_weak = row.downgrade();
        remove.connect_clicked(move |_| {
            if let (Some(this), Some(row)) = (weak.upgrade(), row_weak.upgrade()) {
                this.ranges.remove(&row);
                this.series.borrow_mut().retain(|r| r != &row);
                this.refresh();
            }
        });
        row.add_suffix(&remove);
        let weak = Rc::downgrade(self);
        row.connect_changed(move |_| {
            if let Some(this) = weak.upgrade()
                && !this.updating.get()
            {
                this.refresh();
            }
        });
        let weak = Rc::downgrade(self);
        row.connect_entry_activated(move |_| {
            if let Some(this) = weak.upgrade() {
                this.commit();
            }
        });
        // Before the "Add Series" row, which stays last.
        self.ranges.remove(&self.add_series);
        self.ranges.add_row(&row);
        self.ranges.add_row(&self.add_series);
        self.series.borrow_mut().push(row);
    }

    // --- reacting ---

    /// The range or how it is read changed: read the table again — the shape the cells
    /// suggest when `shape` is `None`, the one the person picked otherwise — and put its
    /// ranges in the rows.
    fn reread(self: &Rc<Self>, shape: Option<ChartShape>) {
        let text = self.range.text();
        let block = grind_sheet::chart::parse_range(&self.app, self.sheet, text.trim())
            .and_then(|qualified| grind_sheet::chart::resolve_range(&self.app, &qualified));
        let Ok((on, start, end)) = block else {
            self.range.add_css_class("error");
            self.preview
                .say(&format!("“{}” is not a range on this sheet", text.trim()));
            self.insert.set_sensitive(false);
            return;
        };
        self.range.remove_css_class("error");
        if on != self.sheet {
            self.preview
                .say("A chart's figures are on the sheet the chart is on");
            self.insert.set_sensitive(false);
            return;
        }
        let Ok(guessed) = self.app.suggest_chart(self.sheet, start, end, shape) else {
            return;
        };
        self.updating.set(true);
        if shape.is_none() {
            self.show_shape(guessed.shape);
        }
        self.show_ranges(&guessed.spec);
        if !self.title_touched.get() {
            self.title
                .set_text(guessed.spec.title.as_deref().unwrap_or_default());
        }
        self.updating.set(false);
        self.block.set(Some((guessed.start, guessed.end)));
        self.refresh();
    }

    /// Everything that follows from the widgets: the preview, the legend's default while
    /// nobody has picked one, which rows apply to this kind, and whether Insert can be pressed.
    fn refresh(&self) {
        let kind = self.kind();
        let pie = kind == ChartKind::Pie;
        self.clockwise.set_visible(pie);
        self.axes.set_visible(!pie);
        for axis in [&self.x, &self.y] {
            axis.expander.set_subtitle(&axis_summary(&axis.read()));
        }
        let mut spec = self.read();
        if !self.legend_touched.get() {
            spec.legend = spec.default_legend();
            self.updating.set(true);
            self.show_legend(spec.legend);
            self.updating.set(false);
        }
        self.ranges.set_subtitle(&ranges_summary(&spec));
        if let Some((start, end)) = self.block.get() {
            self.title_widget
                .set_subtitle(&format!("of {}", block_text(start, end)));
        }
        if spec.series.is_empty() {
            self.preview
                .say("Nothing here is a number to chart — select a table, or type a range");
            self.insert.set_sensitive(false);
            return;
        }
        match self.app.preview_chart(self.sheet, &spec, self.editing) {
            Ok((chart, data)) => {
                self.preview.show(chart, data);
                self.insert.set_sensitive(true);
            }
            Err(error) => {
                self.preview.say(&error.to_string());
                self.insert.set_sensitive(false);
            }
        }
    }

    /// Insert or Apply: one call to the core, one undo step, and the dialog closes. A new chart
    /// goes beside the block it was made from and is scrolled into view.
    fn commit(&self) {
        if !self.insert.is_sensitive() {
            return;
        }
        let spec = self.read();
        let spec = ChartSpec {
            legend: match self.legend_touched.get() {
                true => spec.legend,
                false => spec.default_legend(),
            },
            ..spec
        };
        let result = match self.editing {
            Some(index) => self.app.edit_chart(self.sheet, index, &spec),
            None => {
                let (start, end) = self
                    .block
                    .get()
                    .unwrap_or_else(|| self.grid.selection().rect());
                let (x, y) = self.grid.anchor_beside(start, end);
                self.app
                    .add_chart(self.sheet, &spec, &x, &y, NEW_WIDTH, NEW_HEIGHT)
                    .inspect(|()| {
                        let count = self.app.charts(self.sheet).map_or(0, |c| c.len());
                        self.grid.reveal_chart(count.saturating_sub(1));
                    })
            }
        };
        match result {
            Ok(()) => {
                self.dialog.close();
            }
            Err(error) => self.toasts.add_toast(adw::Toast::new(&error.to_string())),
        }
    }
}

// --- the preview ---------------------------------------------------------------------------

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct Preview {
        pub drawn: RefCell<Option<(Chart, ChartData)>>,
        pub message: RefCell<String>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Preview {
        const NAME: &'static str = "SheetChartPreview";
        type Type = super::Preview;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for Preview {}

    impl WidgetImpl for Preview {
        /// As wide as it is given, and a chart's worth of height — tall enough to read a
        /// chart, short enough to leave the settings under it on screen.
        fn measure(&self, orientation: gtk::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
            match orientation {
                gtk::Orientation::Vertical => (220, 250, -1, -1),
                _ => (240, 480, -1, -1),
            }
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let widget = self.obj();
            let (w, h) = (f64::from(widget.width()), f64::from(widget.height()));
            let palette = crate::theme::Palette::of(&*widget);
            match &*self.drawn.borrow() {
                Some((chart, data)) => {
                    let color = |series: usize, point: Option<usize>| {
                        crate::theme::color(&grind_sheet::effective_color(chart, series, point))
                            .unwrap_or(palette.foreground)
                    };
                    crate::chart::draw(
                        &*widget,
                        snapshot,
                        Rect {
                            x: 0.0,
                            y: 0.0,
                            w,
                            h,
                        },
                        chart,
                        data,
                        &crate::chart::Painter {
                            background: palette.background,
                            // The card is the frame; a second one inside it is a double line.
                            border: gtk::gdk::RGBA::TRANSPARENT,
                            foreground: palette.foreground,
                            grid: palette.lines,
                            color: &color,
                        },
                    );
                }
                None => {
                    let layout = widget.create_pango_layout(Some(&self.message.borrow()));
                    layout.set_width(((w - 48.0).max(1.0) * f64::from(gtk::pango::SCALE)) as i32);
                    layout.set_alignment(gtk::pango::Alignment::Center);
                    layout.set_wrap(gtk::pango::WrapMode::WordChar);
                    let (_, text_h) = layout.pixel_size();
                    snapshot.save();
                    snapshot.translate(&gtk::graphene::Point::new(
                        24.0,
                        ((h - f64::from(text_h)) / 2.0) as f32,
                    ));
                    snapshot.append_layout(
                        &layout,
                        &crate::theme::with_alpha(palette.foreground, 0.55),
                    );
                    snapshot.restore();
                }
            }
        }
    }
}

glib::wrapper! {
    /// The chart a dialog's settings would make, drawn by the grid's own [`crate::chart::draw`]
    /// — or, when they make none, the sentence saying why.
    pub struct Preview(ObjectSubclass<imp::Preview>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Preview {
    fn new() -> Self {
        let preview: Self = glib::Object::new();
        preview.set_hexpand(true);
        preview.update_property(&[gtk::accessible::Property::Label("Chart preview")]);
        preview
    }

    fn show(&self, chart: Chart, data: ChartData) {
        self.imp().drawn.replace(Some((chart, data)));
        self.queue_draw();
    }

    fn say(&self, message: &str) {
        self.imp().drawn.replace(None);
        self.imp().message.replace(message.to_owned());
        self.queue_draw();
    }
}

/// A chart type's glyph: three bars, a rising line, or a pie with a slice cut out — drawn in
/// the button's own ink, so it follows the theme and the pressed state like the label beside it.
fn glyph(kind: ChartKind) -> gtk::DrawingArea {
    let area = gtk::DrawingArea::new();
    area.set_content_width(18);
    area.set_content_height(14);
    area.set_draw_func(move |area, cr, w, h| {
        let ink = crate::theme::Palette::of(area).foreground;
        cr.set_source_rgba(
            f64::from(ink.red()),
            f64::from(ink.green()),
            f64::from(ink.blue()),
            f64::from(ink.alpha()),
        );
        let (w, h) = (f64::from(w), f64::from(h));
        match kind {
            ChartKind::Bar => {
                for (i, height) in [0.55, 1.0, 0.75].into_iter().enumerate() {
                    let bar = w / 4.0;
                    cr.rectangle(
                        i as f64 * (bar + bar / 2.0),
                        h * (1.0 - height),
                        bar,
                        h * height,
                    );
                }
                let _ = cr.fill();
            }
            ChartKind::Line => {
                cr.set_line_width(2.0);
                cr.set_line_cap(gtk::cairo::LineCap::Round);
                cr.set_line_join(gtk::cairo::LineJoin::Round);
                cr.move_to(1.0, h * 0.8);
                cr.line_to(w * 0.35, h * 0.45);
                cr.line_to(w * 0.6, h * 0.65);
                cr.line_to(w - 1.0, h * 0.15);
                let _ = cr.stroke();
            }
            ChartKind::Pie => {
                // Most of a pie, and one slice pulled a little way out of it.
                let (cx, cy, r) = (w / 2.0 - 1.0, h / 2.0 + 0.5, h / 2.0 - 1.5);
                let twelve = -std::f64::consts::FRAC_PI_2;
                let cut = 1.3;
                cr.move_to(cx, cy);
                cr.arc(cx, cy, r, twelve + cut, twelve + std::f64::consts::TAU);
                cr.close_path();
                let _ = cr.fill();
                let middle = twelve + cut / 2.0;
                let (ox, oy) = (cx + 1.8 * middle.cos(), cy + 1.8 * middle.sin());
                cr.move_to(ox, oy);
                cr.arc(ox, oy, r, twelve, twelve + cut);
                cr.close_path();
                let _ = cr.fill();
            }
        }
    });
    area
}

// --- the words -----------------------------------------------------------------------------

/// A block as a person types one: `A1:C5`, or `B2` for one cell.
fn block_text(start: Pos, end: Pos) -> String {
    match start == end {
        true => a1::format(None, start),
        false => format!("{}:{}", a1::format(None, start), a1::format(None, end)),
    }
}

/// A series as its row shows it: `B2:B5=B1`, the vocabulary `chart-add --series` takes.
fn series_text((values, label): &(String, Option<String>)) -> String {
    match label {
        Some(label) => format!("{values}={label}"),
        None => values.clone(),
    }
}

/// A series row read back — its values and the cell naming it; `None` for an empty row.
fn parse_series(text: &str) -> Option<(String, Option<String>)> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    Some(match text.split_once('=') {
        Some((values, label)) if !label.trim().is_empty() => {
            (values.trim().to_owned(), Some(label.trim().to_owned()))
        }
        Some((values, _)) => (values.trim().to_owned(), None),
        None => (text.to_owned(), None),
    })
}

/// A stored range as a person on its own sheet would type it: `Sheet1.B2:Sheet1.B5` on
/// `Sheet1` is `B2:B5`. A range on another sheet keeps its sheet, since that is what it means.
fn plain_range(sheet_name: &str, stored: &str) -> String {
    let Ok(reference) = a1::parse_bracketed(&format!("[{stored}]")) else {
        return stored.to_owned();
    };
    let end = reference
        .end
        .clone()
        .unwrap_or_else(|| reference.start.clone());
    let here = [&reference.start, &end]
        .iter()
        .all(|cell| cell.sheet.as_deref().is_none_or(|s| s == sheet_name));
    let pos = |cell: &grind_sheet::formula::lex::CellRef| {
        Some(Pos::new(cell.row?.index, cell.col?.index))
    };
    match (here, pos(&reference.start), pos(&end)) {
        (true, Some(a), Some(b)) => block_text(a, b),
        _ => stored.to_owned(),
    }
}

/// The block every range of a chart falls in, when they are all on its own sheet — what an
/// *Edit Chart* dialog shows as the chart's range.
fn spanned(app: &App, sheet: usize, spec: &ChartSpec) -> Option<(Pos, Pos)> {
    let mut ranges: Vec<&str> = spec.categories.iter().map(String::as_str).collect();
    for (values, label) in &spec.series {
        ranges.push(values);
        ranges.extend(label.as_deref());
    }
    let mut block: Option<(Pos, Pos)> = None;
    for range in ranges {
        let (on, start, end) = grind_sheet::chart::resolve_range(app, range).ok()?;
        if on != sheet {
            return None;
        }
        block = Some(match block {
            None => (start, end),
            Some((a, b)) => (
                Pos::new(a.row.min(start.row), a.col.min(start.col)),
                Pos::new(b.row.max(end.row), b.col.max(end.col)),
            ),
        });
    }
    block
}

/// The *Ranges* row's subtitle: what is charted, in a phrase.
fn ranges_summary(spec: &ChartSpec) -> String {
    let series = match spec.series.len() {
        1 => "1 series".to_owned(),
        n => format!("{n} series"),
    };
    match &spec.categories {
        Some(categories) => format!("{series}, named by {categories}"),
        None => series,
    }
}

/// An axis row's subtitle: what it carries, in a phrase.
fn axis_summary(axis: &grind_sheet::ChartAxis) -> String {
    let mut parts = Vec::new();
    if let Some(title) = &axis.label {
        parts.push(format!("“{title}”"));
    }
    parts.push(match axis.tick_labels {
        true => "labels".to_owned(),
        false => "no labels".to_owned(),
    });
    if axis.gridlines {
        parts.push("gridlines".to_owned());
    }
    let mut text = parts.join(", ");
    if let Some(first) = text.get(..1) {
        let upper = first.to_uppercase();
        text.replace_range(..1, &upper);
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_series_row_says_what_chart_add_takes_and_reads_it_back() {
        let series = ("B2:B5".to_owned(), Some("B1".to_owned()));
        assert_eq!(series_text(&series), "B2:B5=B1");
        assert_eq!(parse_series("B2:B5=B1"), Some(series));
        assert_eq!(parse_series(" C2:C5 "), Some(("C2:C5".to_owned(), None)));
        assert_eq!(parse_series("C2:C5="), Some(("C2:C5".to_owned(), None)));
        assert_eq!(parse_series("   "), None);
    }

    /// A range on the chart's own sheet is shown the way a person on that sheet would type it;
    /// one on another sheet keeps the sheet, since that is what it means.
    #[test]
    fn a_stored_range_is_shown_as_it_would_be_typed() {
        assert_eq!(plain_range("Sheet1", "Sheet1.B2:Sheet1.B5"), "B2:B5");
        assert_eq!(plain_range("Sheet1", "Sheet1.B1:Sheet1.B1"), "B1");
        assert_eq!(plain_range("Sheet1", "Data.B2:Data.B5"), "Data.B2:Data.B5");
    }

    #[test]
    fn every_legend_position_has_one_entry_and_none_comes_first() {
        assert_eq!(LEGENDS[0].0, None);
        for position in grind_sheet::ChartLegend::ALL {
            assert_eq!(
                LEGENDS.iter().filter(|(l, _)| *l == Some(position)).count(),
                1,
                "{position:?}"
            );
        }
    }

    #[test]
    fn the_subtitles_say_what_is_there_in_a_phrase() {
        let mut spec = ChartSpec::new(ChartKind::Bar);
        spec.series = vec![("B2:B5".into(), None), ("C2:C5".into(), None)];
        spec.categories = Some("A2:A5".into());
        assert_eq!(ranges_summary(&spec), "2 series, named by A2:A5");
        let axis = grind_sheet::ChartAxis {
            label: Some("Votes".into()),
            gridlines: true,
            ..grind_sheet::ChartAxis::default()
        };
        assert_eq!(axis_summary(&axis), "“Votes”, labels, gridlines");
        assert_eq!(axis_summary(&grind_sheet::ChartAxis::bare()), "No labels");
    }
}
