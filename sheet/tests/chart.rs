// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `App::add_chart`/`charts`/`remove_chart`/`chart_data` — the public API, exercised the way
//! `app.rs` exercises everything else. No ODF here; `sheet/src/odf/chart.rs` and
//! `text/tests/image.rs`'s sibling test in this crate cover the file format.

use grind_sheet::{App, ChartAxis, ChartKind, Form};

/// The old positional vocabulary, as the [`grind_sheet::ChartSpec`] `add_chart` and `edit_chart`
/// take now — so a test says what its chart is in one line.
fn spec(
    kind: grind_sheet::ChartKind,
    categories: Option<&str>,
    series: &[(&str, Option<&str>)],
    x_axis: grind_sheet::ChartAxis,
    y_axis: grind_sheet::ChartAxis,
) -> grind_sheet::ChartSpec {
    grind_sheet::ChartSpec {
        categories: categories.map(str::to_owned),
        series: series
            .iter()
            .map(|(values, label)| ((*values).to_owned(), label.map(str::to_owned)))
            .collect(),
        x_axis,
        y_axis,
        ..grind_sheet::ChartSpec::new(kind)
    }
}

/// An axis carrying nothing but a title — what most of these tests want to pass.
fn titled(label: &str) -> ChartAxis {
    ChartAxis {
        label: Some(label.to_owned()),
        ..ChartAxis::default()
    }
}

fn filled(app: &App) {
    let rows = [("GRÜNE", 100.0), ("CDU", 80.0), ("AfD", 60.0)];
    app.set_cell(0, grind_sheet::Pos::new(0, 0), "Party")
        .unwrap();
    app.set_cell(0, grind_sheet::Pos::new(0, 1), "Votes")
        .unwrap();
    for (i, (name, votes)) in rows.iter().enumerate() {
        app.set_cell(0, grind_sheet::Pos::new(i as u32 + 1, 0), *name)
            .unwrap();
        app.set_cell(0, grind_sheet::Pos::new(i as u32 + 1, 1), *votes)
            .unwrap();
    }
}

#[test]
fn a_new_chart_reads_back_the_ranges_it_was_given() {
    let app = App::new();
    filled(&app);
    app.add_chart(
        0,
        &spec(
            ChartKind::Bar,
            Some("A2:A4"),
            &[("B2:B4", Some("B1"))],
            ChartAxis::default(),
            ChartAxis::default(),
        ),
        "1cm",
        "1cm",
        "10cm",
        "8cm",
    )
    .unwrap();

    let charts = app.charts(0).unwrap();
    assert_eq!(charts.len(), 1);
    let chart = &charts[0];
    assert_eq!(chart.kind, ChartKind::Bar);
    // Sheet-qualified, the way ODF's own `table:cell-range-address` always is.
    assert_eq!(chart.categories.as_deref(), Some("Sheet1.A2:Sheet1.A4"));
    assert_eq!(chart.series.len(), 1);
    assert_eq!(chart.series[0].values, "Sheet1.B2:Sheet1.B4");
    assert_eq!(
        chart.series[0].label.as_deref(),
        Some("Sheet1.B1:Sheet1.B1")
    );
}

#[test]
fn chart_data_resolves_against_the_live_sheet() {
    let app = App::new();
    filled(&app);
    app.add_chart(
        0,
        &spec(
            ChartKind::Pie,
            Some("A2:A4"),
            &[("B2:B4", Some("B1"))],
            ChartAxis::default(),
            ChartAxis::default(),
        ),
        "1cm",
        "1cm",
        "10cm",
        "8cm",
    )
    .unwrap();

    let data = app.chart_data(0, 0).unwrap();
    assert_eq!(data.kind, ChartKind::Pie);
    assert_eq!(data.categories, vec!["GRÜNE", "CDU", "AfD"]);
    assert_eq!(data.series.len(), 1);
    assert_eq!(data.series[0].0, "Votes");
    assert_eq!(data.series[0].1, vec![100.0, 80.0, 60.0]);

    // The chart tracks the cells, not a snapshot of them.
    app.set_cell(0, grind_sheet::Pos::new(1, 1), 999.0).unwrap();
    let data = app.chart_data(0, 0).unwrap();
    assert_eq!(data.series[0].1, vec![999.0, 80.0, 60.0]);
}

#[test]
fn removing_a_chart_undoes_back_to_having_it() {
    let app = App::new();
    filled(&app);
    app.add_chart(
        0,
        &spec(
            ChartKind::Line,
            None,
            &[("B2:B4", None)],
            ChartAxis::default(),
            ChartAxis::default(),
        ),
        "1cm",
        "1cm",
        "10cm",
        "8cm",
    )
    .unwrap();
    assert_eq!(app.charts(0).unwrap().len(), 1);

    app.remove_chart(0, 0).unwrap();
    assert_eq!(app.charts(0).unwrap().len(), 0);

    assert!(app.undo());
    assert_eq!(app.charts(0).unwrap().len(), 1);
    assert_eq!(app.charts(0).unwrap()[0].kind, ChartKind::Line);
}

#[test]
fn reshaping_a_chart_moves_it_and_undoes_back() {
    let app = App::new();
    filled(&app);
    app.add_chart(
        0,
        &spec(
            ChartKind::Bar,
            None,
            &[("B2:B4", None)],
            ChartAxis::default(),
            ChartAxis::default(),
        ),
        "1cm",
        "1cm",
        "10cm",
        "8cm",
    )
    .unwrap();

    app.reshape_chart(0, 0, "3cm", "4cm", "12cm", "9cm")
        .unwrap();
    let chart = &app.charts(0).unwrap()[0];
    assert_eq!((chart.x.as_str(), chart.y.as_str()), ("3cm", "4cm"));
    assert_eq!(
        (chart.width.as_str(), chart.height.as_str()),
        ("12cm", "9cm")
    );

    assert!(app.undo());
    let chart = &app.charts(0).unwrap()[0];
    assert_eq!((chart.x.as_str(), chart.y.as_str()), ("1cm", "1cm"));
    assert_eq!(
        (chart.width.as_str(), chart.height.as_str()),
        ("10cm", "8cm")
    );
}

#[test]
fn reshaping_a_chart_that_does_not_exist_is_an_error() {
    let app = App::new();
    filled(&app);
    assert!(app.reshape_chart(0, 0, "1cm", "1cm", "1cm", "1cm").is_err());
}

#[test]
fn a_bad_range_is_an_error_not_a_panic() {
    let app = App::new();
    assert!(
        app.add_chart(
            0,
            &spec(
                ChartKind::Bar,
                None,
                &[("not a range", None)],
                ChartAxis::default(),
                ChartAxis::default()
            ),
            "1cm",
            "1cm",
            "10cm",
            "8cm"
        )
        .is_err()
    );
    assert_eq!(
        app.charts(0).unwrap().len(),
        0,
        "the failed add wrote nothing"
    );
}

/// Adding a chart is not a value edit, so this forces the regenerating writer
/// (`Edits::only_values` goes false — `sheet/src/action.rs`) — the path that has to
/// synthesise the chart's own document from scratch, in both physical forms.
#[test]
fn a_chart_survives_a_save_and_reopen_in_both_forms() {
    for form in [Form::Flat, Form::Package] {
        let app = App::new();
        filled(&app);
        app.add_chart(
            0,
            &spec(
                ChartKind::Pie,
                Some("A2:A4"),
                &[("B2:B4", Some("B1"))],
                titled("Party"),
                titled("Votes"),
            ),
            "1cm",
            "2cm",
            "10cm",
            "8cm",
        )
        .unwrap();

        let bytes = app.save_bytes(form).expect("writes");
        let reopened = App::new();
        reopened
            .open_bytes("test", &bytes)
            .expect("reads its own chart back");

        let charts = reopened.charts(0).unwrap();
        assert_eq!(charts.len(), 1, "{form:?}");
        assert_eq!(charts[0].kind, ChartKind::Pie);
        assert_eq!(charts[0].categories.as_deref(), Some("Sheet1.A2:Sheet1.A4"));
        assert_eq!(charts[0].x_axis.label.as_deref(), Some("Party"));
        assert_eq!(charts[0].y_axis.label.as_deref(), Some("Votes"));
        assert_eq!(charts[0].series[0].values, "Sheet1.B2:Sheet1.B4");
        assert_eq!((charts[0].x.as_str(), charts[0].y.as_str()), ("1cm", "2cm"));

        let data = reopened.chart_data(0, 0).unwrap();
        assert_eq!(data.categories, vec!["GRÜNE", "CDU", "AfD"]);
        assert_eq!(data.series[0].1, vec![100.0, 80.0, 60.0]);
    }
}

/// A user-assigned colour is a sticky override: it survives a save and reopen exactly, even
/// though the writer regenerates the chart's own document from scratch every time
/// (`doc/chart-format.md`).
#[test]
fn a_custom_point_colour_survives_a_save_and_reopen() {
    let app = App::new();
    filled(&app);
    app.add_chart(
        0,
        &spec(
            ChartKind::Bar,
            None,
            &[("B2:B4", None)],
            ChartAxis::default(),
            ChartAxis::default(),
        ),
        "1cm",
        "1cm",
        "10cm",
        "8cm",
    )
    .unwrap();

    let mut series = app.charts(0).unwrap()[0].series.clone();
    series[0].point_colors = vec![None, Some("#123456".to_owned())];
    app.set_chart_style(0, 0, ChartAxis::default(), ChartAxis::default(), series)
        .unwrap();

    let bytes = app.save_bytes(Form::Flat).unwrap();
    let reopened = App::new();
    reopened.open_bytes("test", &bytes).unwrap();
    let charts = reopened.charts(0).unwrap();
    assert_eq!(
        charts[0].series[0]
            .point_colors
            .get(1)
            .cloned()
            .flatten()
            .as_deref(),
        Some("#123456")
    );
    // The point nobody touched is still `None` — still following the default cycle rather
    // than having been made explicit by the round trip.
    assert_eq!(
        charts[0].series[0].point_colors.first().cloned().flatten(),
        None
    );
}

/// Setting a chart's axis labels is one undo step, and clearing one goes back to `None`
/// rather than an empty string.
#[test]
fn set_chart_style_sets_and_undoes_axis_labels() {
    let app = App::new();
    filled(&app);
    app.add_chart(
        0,
        &spec(
            ChartKind::Line,
            None,
            &[("B2:B4", None)],
            ChartAxis::default(),
            ChartAxis::default(),
        ),
        "1cm",
        "1cm",
        "10cm",
        "8cm",
    )
    .unwrap();

    let series = app.charts(0).unwrap()[0].series.clone();
    app.set_chart_style(0, 0, titled("Party"), titled("Votes"), series.clone())
        .unwrap();
    let chart = &app.charts(0).unwrap()[0];
    assert_eq!(chart.x_axis.label.as_deref(), Some("Party"));
    assert_eq!(chart.y_axis.label.as_deref(), Some("Votes"));

    assert!(app.undo());
    let chart = &app.charts(0).unwrap()[0];
    assert_eq!(chart.x_axis.label, None);
    assert_eq!(chart.y_axis.label, None);
}

/// The axis switches are part of the document, not of a shell: they survive a save and a
/// reopen in both physical forms, the same as everything else a chart carries.
#[test]
fn axis_tick_labels_and_gridlines_survive_a_save_and_reopen_in_both_forms() {
    for form in [Form::Flat, Form::Package] {
        let app = App::new();
        filled(&app);
        app.add_chart(
            0,
            &spec(
                ChartKind::Bar,
                Some("A2:A4"),
                &[("B2:B4", None)],
                ChartAxis {
                    label: None,
                    tick_labels: false,
                    gridlines: false,
                },
                ChartAxis {
                    label: Some("Votes".to_owned()),
                    tick_labels: true,
                    gridlines: true,
                },
            ),
            "1cm",
            "1cm",
            "10cm",
            "8cm",
        )
        .unwrap();

        let bytes = app.save_bytes(form).expect("writes");
        let reopened = App::new();
        reopened.open_bytes("test", &bytes).expect("reads back");
        let chart = &reopened.charts(0).unwrap()[0];
        assert!(!chart.x_axis.tick_labels, "{form:?}");
        assert!(!chart.x_axis.gridlines, "{form:?}");
        assert!(chart.y_axis.tick_labels, "{form:?}");
        assert!(chart.y_axis.gridlines, "{form:?}");
        assert_eq!(chart.y_axis.label.as_deref(), Some("Votes"));
    }
}

/// **Both axes always state `chart:display-label`, whichever way it goes.** LibreOffice reads
/// an absent one as `false` (`doc/chart-format.md` has the measurement), so a chart written
/// without it would draw its labels here and not there — writing it always is what keeps the
/// two pictures the same.
#[test]
fn an_axis_with_nothing_said_about_it_reads_back_with_its_labels_shown() {
    let app = App::new();
    filled(&app);
    app.add_chart(
        0,
        &spec(
            ChartKind::Bar,
            Some("A2:A4"),
            &[("B2:B4", None)],
            ChartAxis::default(),
            ChartAxis::default(),
        ),
        "1cm",
        "1cm",
        "10cm",
        "8cm",
    )
    .unwrap();

    let bytes = app.save_bytes(Form::Flat).unwrap();
    let xml = String::from_utf8(bytes.clone()).unwrap();
    assert_eq!(
        xml.matches("chart:display-label=\"true\"").count(),
        2,
        "one per axis, stated rather than left to a default"
    );

    let reopened = App::new();
    reopened.open_bytes("test", &bytes).unwrap();
    let chart = &reopened.charts(0).unwrap()[0];
    assert!(chart.x_axis.tick_labels);
    assert!(chart.y_axis.tick_labels);
}

/// `edit_chart` changes what a chart is, in the vocabulary a user types, and undoes in one
/// step — including the kind, which nothing else could change short of deleting the chart.
#[test]
fn editing_a_chart_changes_its_kind_and_ranges_and_undoes_in_one_step() {
    let app = App::new();
    filled(&app);
    app.add_chart(
        0,
        &spec(
            ChartKind::Bar,
            Some("A2:A4"),
            &[("B2:B4", None)],
            ChartAxis::default(),
            ChartAxis::default(),
        ),
        "1cm",
        "1cm",
        "10cm",
        "8cm",
    )
    .unwrap();

    app.edit_chart(
        0,
        0,
        &spec(
            ChartKind::Line,
            Some("A2:A3"),
            &[("B2:B3", Some("B1"))],
            titled("Party"),
            ChartAxis::default(),
        ),
    )
    .unwrap();

    let chart = &app.charts(0).unwrap()[0];
    assert_eq!(chart.kind, ChartKind::Line);
    assert_eq!(chart.categories.as_deref(), Some("Sheet1.A2:Sheet1.A3"));
    assert_eq!(chart.series[0].values, "Sheet1.B2:Sheet1.B3");
    assert_eq!(chart.x_axis.label.as_deref(), Some("Party"));
    // The position is `reshape_chart`'s, and an edit leaves it exactly where a drag put it.
    assert_eq!((chart.x.as_str(), chart.y.as_str()), ("1cm", "1cm"));

    assert!(app.undo());
    let chart = &app.charts(0).unwrap()[0];
    assert_eq!(chart.kind, ChartKind::Bar);
    assert_eq!(chart.series[0].values, "Sheet1.B2:Sheet1.B4");
    assert_eq!(chart.x_axis.label, None);
}

/// A colour picked by hand is matched back on by *range*, not by position — so inserting a
/// series above the one it was picked on does not shuffle it down onto a different line.
#[test]
fn editing_a_chart_keeps_a_hand_picked_colour_on_the_series_it_was_picked_on() {
    let app = App::new();
    filled(&app);
    app.add_chart(
        0,
        &spec(
            ChartKind::Line,
            None,
            &[("B2:B4", None)],
            ChartAxis::default(),
            ChartAxis::default(),
        ),
        "1cm",
        "1cm",
        "10cm",
        "8cm",
    )
    .unwrap();
    let mut series = app.charts(0).unwrap()[0].series.clone();
    series[0].color = Some("#123456".to_owned());
    app.set_chart_style(0, 0, ChartAxis::default(), ChartAxis::default(), series)
        .unwrap();

    // A new series *ahead* of the coloured one: it is now series 1, and the colour came with
    // it rather than staying on index 0.
    app.edit_chart(
        0,
        0,
        &spec(
            ChartKind::Line,
            None,
            &[("A2:A4", None), ("B2:B4", None)],
            ChartAxis::default(),
            ChartAxis::default(),
        ),
    )
    .unwrap();
    let chart = &app.charts(0).unwrap()[0];
    assert_eq!(chart.series.len(), 2);
    assert_eq!(chart.series[0].color, None);
    assert_eq!(chart.series[1].color.as_deref(), Some("#123456"));

    // A series pointed somewhere else starts again from the default cycle.
    app.edit_chart(
        0,
        0,
        &spec(
            ChartKind::Line,
            None,
            &[("B2:B3", None)],
            ChartAxis::default(),
            ChartAxis::default(),
        ),
    )
    .unwrap();
    assert_eq!(app.charts(0).unwrap()[0].series[0].color, None);
}

/// Editing a chart that is not there is an error, not a panic — the same as reshaping one.
#[test]
fn editing_a_chart_that_does_not_exist_is_an_error() {
    let app = App::new();
    filled(&app);
    assert!(
        app.edit_chart(
            0,
            0,
            &spec(
                ChartKind::Bar,
                None,
                &[("B2:B4", None)],
                ChartAxis::default(),
                ChartAxis::default()
            )
        )
        .is_err()
    );
}

/// A chart over [`filled`]'s table, at a fixed place — the tests below care about what the
/// chart *is*, not where it sits, so the one call that says where lives here.
fn add(app: &App, kind: ChartKind, series: &[(&str, Option<&str>)]) {
    app.add_chart(
        0,
        &spec(
            kind,
            Some("A2:A4"),
            series,
            ChartAxis::default(),
            ChartAxis::default(),
        ),
        "1cm",
        "1cm",
        "10cm",
        "8cm",
    )
    .unwrap();
}

/// A document written and read back in `form`.
fn reopened(app: &App, form: Form) -> App {
    let bytes = app.save_bytes(form).expect("writes");
    let again = App::new();
    again
        .open_bytes("test", &bytes)
        .expect("reads its own chart back");
    again
}

/// `doc/chart-format.md`, Direction: a new pie runs clockwise, and a pie turned round keeps
/// running the way it was turned — both through a save, in both physical forms.
#[test]
fn a_pies_direction_survives_a_save_and_reopen_either_way_round() {
    for form in [Form::Flat, Form::Package] {
        let app = App::new();
        filled(&app);
        add(&app, ChartKind::Pie, &[("B2:B4", None)]);
        assert!(
            app.charts(0).unwrap()[0].clockwise,
            "a new pie is clockwise"
        );
        assert!(
            reopened(&app, form).charts(0).unwrap()[0].clockwise,
            "{form:?}"
        );

        // Turned round the only way a file can say so, then carried through a save.
        let flat = String::from_utf8(app.save_bytes(Form::Flat).unwrap()).unwrap();
        let counter = flat.replace(
            "chart:reverse-direction=\"true\"",
            "chart:reverse-direction=\"false\"",
        );
        let back = App::new();
        back.open_bytes("test.fods", counter.as_bytes()).unwrap();
        assert!(!back.charts(0).unwrap()[0].clockwise);
        assert!(
            !reopened(&back, form).charts(0).unwrap()[0].clockwise,
            "a counter-clockwise pie stays one ({form:?})"
        );
    }
}

/// A pie whose file says nothing about its direction reads as LibreOffice draws it —
/// counter-clockwise — rather than as this build's own default, which is what made the same
/// bytes draw two different pies in two programs.
#[test]
fn a_pie_whose_file_says_nothing_reads_counter_clockwise() {
    let app = App::new();
    filled(&app);
    add(&app, ChartKind::Pie, &[("B2:B4", None)]);
    let flat = String::from_utf8(app.save_bytes(Form::Flat).unwrap()).unwrap();
    let silent = flat.replace(" chart:reverse-direction=\"true\"", "");
    assert_ne!(silent, flat, "the writer states the direction");
    let back = App::new();
    back.open_bytes("test.fods", silent.as_bytes()).unwrap();
    assert!(!back.charts(0).unwrap()[0].clockwise);
}

/// The measurement behind `doc/chart-format.md`'s Colour section: LibreOffice ignores a data
/// point's style under a series that names none, so every series written names one.
#[test]
fn every_series_written_names_a_style_of_its_own() {
    for kind in [ChartKind::Bar, ChartKind::Line, ChartKind::Pie] {
        let app = App::new();
        filled(&app);
        add(&app, kind, &[("B2:B4", Some("B1")), ("B2:B4", None)]);
        let flat = String::from_utf8(app.save_bytes(Form::Flat).unwrap()).unwrap();
        let series: Vec<&str> = flat
            .split("<chart:series ")
            .skip(1)
            .map(|rest| &rest[..rest.find('>').unwrap()])
            .collect();
        assert_eq!(series.len(), 2, "{kind:?}");
        for attributes in series {
            assert!(
                attributes.contains("chart:style-name=\""),
                "{kind:?}: a series with no style of its own: {attributes}"
            );
        }
    }
}

/// A bar series picked a colour by hand keeps it, and one nobody touched comes back with no
/// override — still following the cycle, so adding a series in front of it still moves it on.
#[test]
fn a_bar_series_colour_survives_a_save_and_an_untouched_one_stays_untouched() {
    let app = App::new();
    filled(&app);
    add(
        &app,
        ChartKind::Bar,
        &[("B2:B4", Some("B1")), ("B2:B4", None)],
    );
    let mut series = app.charts(0).unwrap()[0].series.clone();
    series[1].color = Some("#abcdef".to_owned());
    app.set_chart_style(0, 0, ChartAxis::default(), ChartAxis::default(), series)
        .unwrap();
    for form in [Form::Flat, Form::Package] {
        let back = reopened(&app, form);
        let series = &back.charts(0).unwrap()[0].series;
        assert_eq!(series[0].color, None, "{form:?}");
        assert!(
            series[0].point_colors.iter().all(Option::is_none),
            "{form:?}"
        );
        assert_eq!(series[1].color.as_deref(), Some("#abcdef"), "{form:?}");
        assert!(
            series[1].point_colors.iter().all(Option::is_none),
            "{form:?}"
        );
    }
}

/// A bar chart this build wrote *before* a colour was a series — every bar its own data-point
/// style cycling per point, the series naming no style — reads back untouched rather than with
/// every bar's old default mistaken for a colour somebody picked. The bytes are that writer's
/// own shape, spelled out so the test does not depend on a writer that no longer exists.
#[test]
fn a_bar_chart_written_per_point_by_an_older_build_reads_back_untouched() {
    let app = App::new();
    filled(&app);
    add(&app, ChartKind::Bar, &[("B2:B4", None)]);
    let flat = String::from_utf8(app.save_bytes(Form::Flat).unwrap()).unwrap();
    let cycle = [
        grind_sheet::series_color(0),
        grind_sheet::series_color(1),
        grind_sheet::series_color(2),
    ];
    let styles: String = cycle
        .iter()
        .enumerate()
        .map(|(point, hex)| {
            format!(
                "<style:style style:name=\"old0-{point}\" style:family=\"chart\">\
                 <style:graphic-properties svg:stroke-color=\"{hex}\" draw:fill-color=\"{hex}\"/>\
                 </style:style>"
            )
        })
        .collect();
    let points: String = (0..3)
        .map(|point| format!("<chart:data-point chart:style-name=\"old0-{point}\"/>"))
        .collect();
    let start = flat.find("<chart:series ").unwrap();
    let end = flat.find("</chart:series>").unwrap() + "</chart:series>".len();
    let old_series = format!(
        "<chart:series chart:class=\"chart:bar\" \
         chart:values-cell-range-address=\"Sheet1.B2:Sheet1.B4\">{points}</chart:series>"
    );
    let old = format!("{}{old_series}{}", &flat[..start], &flat[end..]).replace(
        "<office:automatic-styles>\n         <style:style style:name=\"gch0\"",
        &format!("<office:automatic-styles>{styles}<style:style style:name=\"gch0\""),
    );
    assert!(old.contains("old0-2"), "the old styles went in");
    let back = App::new();
    back.open_bytes("test.fods", old.as_bytes()).unwrap();
    let series = &back.charts(0).unwrap()[0].series[0];
    assert_eq!(series.color, None);
    assert!(
        series.point_colors.iter().all(Option::is_none),
        "an old default read as an override: {:?}",
        series.point_colors
    );
}

/// A chart's own title and legend (`doc/chart-format.md`, The chart's own title and legend):
/// written, read back in both forms, and cleared again by an edit that says so.
#[test]
fn a_charts_own_title_and_legend_survive_a_save_and_an_edit_clears_them() {
    let app = App::new();
    filled(&app);
    let titled = grind_sheet::ChartSpec {
        title: Some("Votes, 2026".to_owned()),
        legend: Some(grind_sheet::ChartLegend::Bottom),
        ..spec(
            ChartKind::Bar,
            Some("A2:A4"),
            &[("B2:B4", Some("B1"))],
            ChartAxis::default(),
            ChartAxis::default(),
        )
    };
    app.add_chart(0, &titled, "1cm", "1cm", "10cm", "8cm")
        .unwrap();
    for form in [Form::Flat, Form::Package] {
        let chart = reopened(&app, form).charts(0).unwrap()[0].clone();
        assert_eq!(chart.title.as_deref(), Some("Votes, 2026"), "{form:?}");
        assert_eq!(
            chart.legend,
            Some(grind_sheet::ChartLegend::Bottom),
            "{form:?}"
        );
    }
    // An empty title is no title, and no legend is no `chart:legend` at all.
    let bare = grind_sheet::ChartSpec {
        title: Some("  ".to_owned()),
        legend: None,
        ..titled
    };
    app.edit_chart(0, 0, &bare).unwrap();
    let chart = &app.charts(0).unwrap()[0];
    assert_eq!((chart.title.as_deref(), chart.legend), (None, None));
    let flat = String::from_utf8(app.save_bytes(Form::Flat).unwrap()).unwrap();
    assert!(!flat.contains("chart:legend") && !flat.contains("Votes, 2026"));
    assert!(app.undo(), "the edit is one step");
    assert_eq!(
        app.charts(0).unwrap()[0].title.as_deref(),
        Some("Votes, 2026")
    );
}

/// A legend LibreOffice put in a corner reads as the edge it is on, and one that names no
/// position at all reads as the end, where LibreOffice draws it.
#[test]
fn a_legend_in_a_corner_or_nowhere_in_particular_reads_as_an_edge() {
    let app = App::new();
    filled(&app);
    let legend = grind_sheet::ChartSpec {
        legend: Some(grind_sheet::ChartLegend::End),
        ..spec(
            ChartKind::Pie,
            Some("A2:A4"),
            &[("B2:B4", None)],
            ChartAxis::default(),
            ChartAxis::default(),
        )
    };
    app.add_chart(0, &legend, "1cm", "1cm", "10cm", "8cm")
        .unwrap();
    let flat = String::from_utf8(app.save_bytes(Form::Flat).unwrap()).unwrap();
    for (written, read) in [
        (
            "chart:legend-position=\"top-start\"",
            grind_sheet::ChartLegend::Top,
        ),
        (
            "chart:legend-position=\"bottom-end\"",
            grind_sheet::ChartLegend::Bottom,
        ),
        ("", grind_sheet::ChartLegend::End),
    ] {
        let bytes = flat.replace("chart:legend-position=\"end\"", written);
        let back = App::new();
        back.open_bytes("test.fods", bytes.as_bytes()).unwrap();
        assert_eq!(back.charts(0).unwrap()[0].legend, Some(read), "{written:?}");
    }
}
