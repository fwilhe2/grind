// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `App::add_chart`/`charts`/`remove_chart`/`chart_data` — the public API, exercised the way
//! `app.rs` exercises everything else. No ODF here; `sheet/src/odf/chart.rs` and
//! `text/tests/image.rs`'s sibling test in this crate cover the file format.

use grind_sheet::{App, ChartAxis, ChartKind, Form};

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
        ChartKind::Bar,
        Some("A2:A4"),
        &[("B2:B4", Some("B1"))],
        "1cm",
        "1cm",
        "10cm",
        "8cm",
        ChartAxis::default(),
        ChartAxis::default(),
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
        ChartKind::Pie,
        Some("A2:A4"),
        &[("B2:B4", Some("B1"))],
        "1cm",
        "1cm",
        "10cm",
        "8cm",
        ChartAxis::default(),
        ChartAxis::default(),
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
        ChartKind::Line,
        None,
        &[("B2:B4", None)],
        "1cm",
        "1cm",
        "10cm",
        "8cm",
        ChartAxis::default(),
        ChartAxis::default(),
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
        ChartKind::Bar,
        None,
        &[("B2:B4", None)],
        "1cm",
        "1cm",
        "10cm",
        "8cm",
        ChartAxis::default(),
        ChartAxis::default(),
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
            ChartKind::Bar,
            None,
            &[("not a range", None)],
            "1cm",
            "1cm",
            "10cm",
            "8cm",
            ChartAxis::default(),
            ChartAxis::default(),
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
            ChartKind::Pie,
            Some("A2:A4"),
            &[("B2:B4", Some("B1"))],
            "1cm",
            "2cm",
            "10cm",
            "8cm",
            titled("Party"),
            titled("Votes"),
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
        ChartKind::Bar,
        None,
        &[("B2:B4", None)],
        "1cm",
        "1cm",
        "10cm",
        "8cm",
        ChartAxis::default(),
        ChartAxis::default(),
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
        ChartKind::Line,
        None,
        &[("B2:B4", None)],
        "1cm",
        "1cm",
        "10cm",
        "8cm",
        ChartAxis::default(),
        ChartAxis::default(),
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
            ChartKind::Bar,
            Some("A2:A4"),
            &[("B2:B4", None)],
            "1cm",
            "1cm",
            "10cm",
            "8cm",
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
        ChartKind::Bar,
        Some("A2:A4"),
        &[("B2:B4", None)],
        "1cm",
        "1cm",
        "10cm",
        "8cm",
        ChartAxis::default(),
        ChartAxis::default(),
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
        ChartKind::Bar,
        Some("A2:A4"),
        &[("B2:B4", None)],
        "1cm",
        "1cm",
        "10cm",
        "8cm",
        ChartAxis::default(),
        ChartAxis::default(),
    )
    .unwrap();

    app.edit_chart(
        0,
        0,
        ChartKind::Line,
        Some("A2:A3"),
        &[("B2:B3", Some("B1"))],
        titled("Party"),
        ChartAxis::default(),
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
        ChartKind::Line,
        None,
        &[("B2:B4", None)],
        "1cm",
        "1cm",
        "10cm",
        "8cm",
        ChartAxis::default(),
        ChartAxis::default(),
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
        ChartKind::Line,
        None,
        &[("A2:A4", None), ("B2:B4", None)],
        ChartAxis::default(),
        ChartAxis::default(),
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
        ChartKind::Line,
        None,
        &[("B2:B3", None)],
        ChartAxis::default(),
        ChartAxis::default(),
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
            ChartKind::Bar,
            None,
            &[("B2:B4", None)],
            ChartAxis::default(),
            ChartAxis::default(),
        )
        .is_err()
    );
}
