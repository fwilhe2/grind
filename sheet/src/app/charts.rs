// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later
//! Charts: adding, reshaping, restyling and removing them, and the data they draw.

use crate::*;

impl App {
    /// Add a chart at the given place (`x`/`y`/`width`/`height` are ODF lengths), resolving the
    /// spec's ranges as a formula's references resolve ([`chart::parse_range`]). One undo step.
    pub fn add_chart(
        &self,
        sheet: usize,
        spec: &chart::Spec,
        x: &str,
        y: &str,
        width: &str,
        height: &str,
    ) -> Result<()> {
        let mut chart = self.charted(sheet, spec, None)?;
        (chart.x, chart.y, chart.width, chart.height) = (
            x.to_owned(),
            y.to_owned(),
            width.to_owned(),
            height.to_owned(),
        );

        self.mutate(|state| {
            let sheet_len = state
                .doc
                .sheet(sheet)
                .ok_or(Error::NoSuchSheet(sheet))?
                .charts()
                .len();
            let inverse = state
                .doc
                .apply(Action::InsertChart {
                    sheet,
                    index: sheet_len,
                    chart: Box::new(chart),
                })
                .ok_or(Error::NoSuchSheet(sheet))?;
            state.undo.push(inverse);
            state.redo.clear();
            Ok(())
        })
    }

    /// The chart a block of cells makes ([`chart::guess`]). `shape` overrides which way the series
    /// run and whether the first row or column names things. Nothing is written;
    /// [`Self::add_chart`] takes the result.
    pub fn suggest_chart(
        &self,
        sheet: usize,
        start: Pos,
        end: Pos,
        shape: Option<chart::Shape>,
    ) -> Result<chart::Guess> {
        let state = self.state.read().unwrap();
        let s = state.doc.sheet(sheet).ok_or(Error::NoSuchSheet(sheet))?;
        Ok(chart::guess(s, start, end, shape, state.doc.null_date))
    }

    /// Every chart on a sheet, in document order.
    pub fn charts(&self, sheet: usize) -> Result<Vec<Chart>> {
        let state = self.state.read().unwrap();
        let s = state.doc.sheet(sheet).ok_or(Error::NoSuchSheet(sheet))?;
        Ok(s.charts().to_vec())
    }

    /// Remove the chart at `index` on `sheet`.
    pub fn remove_chart(&self, sheet: usize, index: usize) -> Result<()> {
        self.mutate(|state| {
            let inverse = state
                .doc
                .apply(Action::RemoveChart { sheet, index })
                .ok_or(Error::NoSuchSheet(sheet))?;
            state.undo.push(inverse);
            state.redo.clear();
            Ok(())
        })
    }

    /// Move or resize the chart at `index` (ODF lengths), one undo step per call — a shell calls
    /// it once, on release.
    pub fn reshape_chart(
        &self,
        sheet: usize,
        index: usize,
        x: &str,
        y: &str,
        width: &str,
        height: &str,
    ) -> Result<()> {
        self.mutate(|state| {
            let inverse = state
                .doc
                .apply(Action::ReshapeChart {
                    sheet,
                    index,
                    x: x.to_owned(),
                    y: y.to_owned(),
                    width: width.to_owned(),
                    height: height.to_owned(),
                })
                .ok_or_else(|| Error::BadSheet(format!("sheet {sheet} has no chart {index}")))?;
            state.undo.push(inverse);
            state.redo.clear();
            Ok(())
        })
    }

    /// Replace a chart's axes and resolved series (colours included) as one undo step: read it
    /// through [`Self::charts`], change a field, send it back. To change what a chart points at,
    /// in the ranges a user types, use [`Self::edit_chart`].
    pub fn set_chart_style(
        &self,
        sheet: usize,
        index: usize,
        x_axis: chart::Axis,
        y_axis: chart::Axis,
        series: Vec<chart::Series>,
    ) -> Result<()> {
        let mut chart = self.chart(sheet, index)?;
        chart.x_axis = x_axis;
        chart.y_axis = y_axis;
        chart.series = series;
        self.replace_chart(sheet, index, chart)
    }

    /// Change what a chart is — everything a [`chart::Spec`] says — as one undo step, leaving its
    /// position and size alone. Hand-picked colours survive on series still pointing at the same
    /// range (matched by address, not position); a series whose range changed starts from the
    /// default cycle.
    pub fn edit_chart(&self, sheet: usize, index: usize, spec: &chart::Spec) -> Result<()> {
        let chart = self.charted(sheet, spec, Some(index))?;
        self.replace_chart(sheet, index, chart)
    }

    /// What a spec would draw and the data behind it, nothing written — exactly what
    /// [`Self::add_chart`], or with `editing` [`Self::edit_chart`], would make. A chart dialog's
    /// preview.
    pub fn preview_chart(
        &self,
        sheet: usize,
        spec: &chart::Spec,
        editing: Option<usize>,
    ) -> Result<(Chart, ChartData)> {
        let chart = self.charted(sheet, spec, editing)?;
        let data = ChartData::read(self, &chart)?;
        Ok((chart, data))
    }

    /// The chart a spec makes, new or changed from the chart at `editing`, shared by
    /// [`Self::add_chart`], [`Self::edit_chart`] and [`Self::preview_chart`].
    fn charted(&self, sheet: usize, spec: &chart::Spec, editing: Option<usize>) -> Result<Chart> {
        let old = editing.map(|index| self.chart(sheet, index)).transpose()?;
        let mut chart = old.clone().unwrap_or_else(|| {
            Chart::new(
                spec.kind,
                "0cm".to_owned(),
                "0cm".to_owned(),
                "10cm".to_owned(),
                "8cm".to_owned(),
            )
        });
        self.fill_chart(sheet, spec, &mut chart)?;
        chart.kind = spec.kind;
        if let Some(old) = old {
            for new in &mut chart.series {
                if let Some(old) = old.series.iter().find(|old| old.values == new.values) {
                    new.color = old.color.clone();
                    new.point_colors = old.point_colors.clone();
                }
            }
        }
        Ok(chart)
    }

    /// Everything a spec says, resolved into `chart` — before the mutation, since resolving takes
    /// the same lock.
    fn fill_chart(&self, sheet: usize, spec: &chart::Spec, chart: &mut Chart) -> Result<()> {
        let series: Vec<(&str, Option<&str>)> = spec
            .series
            .iter()
            .map(|(values, label)| (values.as_str(), label.as_deref()))
            .collect();
        let (categories, series) =
            self.resolve_chart_ranges(sheet, spec.categories.as_deref(), &series)?;
        chart.categories = categories;
        chart.series = series;
        chart.x_axis = spec.x_axis.clone();
        chart.y_axis = spec.y_axis.clone();
        chart.clockwise = spec.clockwise;
        chart.title = spec
            .title
            .as_deref()
            .map(str::trim)
            .filter(|title| !title.is_empty())
            .map(str::to_owned);
        chart.legend = spec.legend;
        Ok(())
    }

    /// One chart, by index.
    fn chart(&self, sheet: usize, index: usize) -> Result<Chart> {
        let state = self.state.read().unwrap();
        let s = state.doc.sheet(sheet).ok_or(Error::NoSuchSheet(sheet))?;
        s.charts()
            .get(index)
            .cloned()
            .ok_or_else(|| Error::BadSheet(format!("sheet {sheet} has no chart {index}")))
    }

    /// The write half of the same pair: one `Action::ReplaceChart`, one undo entry.
    fn replace_chart(&self, sheet: usize, index: usize, chart: Chart) -> Result<()> {
        self.mutate(|state| {
            let inverse = state
                .doc
                .apply(Action::ReplaceChart {
                    sheet,
                    index,
                    chart: Box::new(chart),
                })
                .ok_or_else(|| Error::BadSheet(format!("sheet {sheet} has no chart {index}")))?;
            state.undo.push(inverse);
            state.redo.clear();
            Ok(())
        })
    }

    /// A chart's ranges as a user types them, resolved into the addresses a chart stores — before
    /// the mutation, since resolving takes the same lock.
    fn resolve_chart_ranges(
        &self,
        sheet: usize,
        categories: Option<&str>,
        series: &[(&str, Option<&str>)],
    ) -> Result<(Option<String>, Vec<chart::Series>)> {
        let categories = categories
            .map(|addr| chart::parse_range(self, sheet, addr))
            .transpose()?;
        let series = series
            .iter()
            .map(|(values, label)| {
                Ok(chart::Series {
                    values: chart::parse_range(self, sheet, values)?,
                    label: label
                        .map(|addr| chart::parse_range(self, sheet, addr))
                        .transpose()?,
                    color: None,
                    point_colors: Vec::new(),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok((categories, series))
    }

    /// A chart's data, resolved against the live sheet — what a shell draws from.
    pub fn chart_data(&self, sheet: usize, index: usize) -> Result<ChartData> {
        // Read and released before `ChartData::read` takes the lock again for the cells.
        let chart = self.chart(sheet, index)?;
        ChartData::read(self, &chart)
    }
}
