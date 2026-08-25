// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `App::add_chart`/`charts`/`remove_chart`/`chart_data` — the public API, exercised the way
//! `app.rs` exercises everything else. No ODF here; `sheet/src/odf/chart.rs` and
//! `text/tests/image.rs`'s sibling test in this crate cover the file format.

use grind_sheet::{App, ChartKind, Form};

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
        None,
        None,
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
        None,
        None,
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
        None,
        None,
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
        None,
        None,
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
            None,
            None,
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
            Some("Party"),
            Some("Votes"),
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
        assert_eq!(charts[0].x_axis_label.as_deref(), Some("Party"));
        assert_eq!(charts[0].y_axis_label.as_deref(), Some("Votes"));
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
        None,
        None,
    )
    .unwrap();

    let mut series = app.charts(0).unwrap()[0].series.clone();
    series[0].point_colors = vec![None, Some("#123456".to_owned())];
    app.set_chart_style(0, 0, None, None, series).unwrap();

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
        None,
        None,
    )
    .unwrap();

    let series = app.charts(0).unwrap()[0].series.clone();
    app.set_chart_style(0, 0, Some("Party"), Some("Votes"), series.clone())
        .unwrap();
    let chart = &app.charts(0).unwrap()[0];
    assert_eq!(chart.x_axis_label.as_deref(), Some("Party"));
    assert_eq!(chart.y_axis_label.as_deref(), Some("Votes"));

    assert!(app.undo());
    let chart = &app.charts(0).unwrap()[0];
    assert_eq!(chart.x_axis_label, None);
    assert_eq!(chart.y_axis_label, None);
}
