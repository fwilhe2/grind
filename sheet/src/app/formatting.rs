// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later
//! How cells and tracks look: number formats, cell styles, column widths, row heights, and
//! hidden tracks.

use crate::*;

impl App {
    /// Set — or with `None`, clear — the number format of every cell in a rectangle, as one undo
    /// step. Returns how many cells were touched.
    ///
    /// Bounded by [`MAX_FORMATTED_CELLS`]: a format costs an entry per cell, so `A1:Z1000000` is
    /// refused. A whole column (`A:A`) is resolved against the used extent by the caller first.
    pub fn set_format(
        &self,
        sheet: usize,
        start: Pos,
        end: Pos,
        format: Option<numfmt::Format>,
    ) -> Result<usize> {
        let cells = self.rectangle(start, end)?;
        self.mutate(|state| {
            if state.doc.sheet(sheet).is_none() {
                return Err(Error::NoSuchSheet(sheet));
            }
            // A cell that already has the format is left out, so re-applying one pushes no empty
            // undo entry.
            let updates = cells
                .filter(|pos| {
                    state.doc.sheet(sheet).and_then(|s| s.format(*pos)) != format.as_ref()
                })
                .map(|pos| Action::SetFormat {
                    sheet,
                    pos,
                    format: format.clone().map(Box::new),
                })
                .collect::<Vec<_>>();
            self::apply_batch(state, sheet, updates)
        })
    }

    /// Set — or with `None`, clear — the styling of every cell in a rectangle, as
    /// [`App::set_format`] does. It replaces rather than merges: "bold as well" reads the style
    /// first ([`App::style_at`]).
    pub fn set_style(
        &self,
        sheet: usize,
        start: Pos,
        end: Pos,
        style: Option<style::CellStyle>,
    ) -> Result<usize> {
        let cells = self.rectangle(start, end)?;
        // A style that sets nothing is no style, so both spellings of "plain" give the same
        // document.
        let style = style.filter(|s| !s.is_plain());
        self.mutate(|state| {
            if state.doc.sheet(sheet).is_none() {
                return Err(Error::NoSuchSheet(sheet));
            }
            let updates = cells
                .filter(|pos| state.doc.sheet(sheet).and_then(|s| s.style(*pos)) != style.as_ref())
                .map(|pos| Action::SetStyle {
                    sheet,
                    pos,
                    style: style.clone().map(Box::new),
                })
                .collect::<Vec<_>>();
            self::apply_batch(state, sheet, updates)
        })
    }

    /// Set — or with `None`, clear — the width of a run of columns (§5.4), as one undo step,
    /// bounded by [`MAX_TRACK_RUN`]. The width is an ODF length (`"2.5cm"`, `"64pt"`), checked
    /// here: an unparseable one would be a column no renderer draws. `style::mm_length` spells a
    /// measured one.
    pub fn set_col_width(
        &self,
        sheet: usize,
        cols: Range<u32>,
        width: Option<String>,
    ) -> Result<usize> {
        let width = self.track_size(cols.len() as u64, width)?;
        self.mutate(|state| {
            if state.doc.sheet(sheet).is_none() {
                return Err(Error::NoSuchSheet(sheet));
            }
            let updates = cols
                .filter(|col| {
                    state.doc.sheet(sheet).and_then(|s| s.col_width(*col)) != width.as_deref()
                })
                .map(|col| Action::SetColWidth {
                    sheet,
                    col,
                    width: width.clone(),
                })
                .collect::<Vec<_>>();
            self::apply_batch(state, sheet, updates)
        })
    }

    /// The row twin of [`App::set_col_width`], in every respect.
    pub fn set_row_height(
        &self,
        sheet: usize,
        rows: Range<u32>,
        height: Option<String>,
    ) -> Result<usize> {
        let height = self.track_size(rows.len() as u64, height)?;
        self.mutate(|state| {
            if state.doc.sheet(sheet).is_none() {
                return Err(Error::NoSuchSheet(sheet));
            }
            let updates = rows
                .filter(|row| {
                    state.doc.sheet(sheet).and_then(|s| s.row_height(*row)) != height.as_deref()
                })
                .map(|row| Action::SetRowHeight {
                    sheet,
                    row,
                    height: height.clone(),
                    measured: false,
                })
                .collect::<Vec<_>>();
            self::apply_batch(state, sheet, updates)
        })
    }

    /// Fit tracks to their content, as one undo step: each column of `widths` set to its width
    /// (`None` for the default), and every row in `rows` that has a height of its own given it
    /// back. The caller measures widths in its own font, with [`fit::widest`]. Rows without a
    /// height are untouched, so `rows` may be the whole sheet.
    pub fn fit(
        &self,
        sheet: usize,
        widths: Vec<(u32, Option<String>)>,
        rows: Range<u32>,
    ) -> Result<usize> {
        if widths.len() as u64 > u64::from(MAX_TRACK_RUN) {
            return Err(Error::TooLarge(widths.len() as u64));
        }
        let widths = widths
            .into_iter()
            .map(|(col, width)| Ok((col, self.track_size(1, width)?)))
            .collect::<Result<Vec<_>>>()?;
        self.mutate(|state| {
            let Some(s) = state.doc.sheet(sheet) else {
                return Err(Error::NoSuchSheet(sheet));
            };
            let mut updates = widths
                .into_iter()
                .filter(|(col, width)| s.col_width(*col) != width.as_deref())
                .map(|(col, width)| Action::SetColWidth { sheet, col, width })
                .collect::<Vec<_>>();
            updates.extend(
                s.row_heights()
                    .map(|(row, _)| row)
                    .filter(|row| rows.contains(row))
                    .map(|row| Action::SetRowHeight {
                        sheet,
                        row,
                        height: None,
                        measured: false,
                    }),
            );
            self::apply_batch(state, sheet, updates)
        })
    }

    /// Hide — or with `hidden: false`, show — a run of columns by hand
    /// (`table:visibility="collapse"`), as one undo step bounded by [`MAX_TRACK_RUN`].
    pub fn set_col_hidden(&self, sheet: usize, cols: Range<u32>, hidden: bool) -> Result<usize> {
        if cols.len() as u64 > u64::from(MAX_TRACK_RUN) {
            return Err(Error::TooLarge(cols.len() as u64));
        }
        self.mutate(|state| {
            if state.doc.sheet(sheet).is_none() {
                return Err(Error::NoSuchSheet(sheet));
            }
            let updates = cols
                .filter(|col| {
                    state
                        .doc
                        .sheet(sheet)
                        .is_some_and(|s| s.col_hidden(*col) != hidden)
                })
                .map(|col| Action::SetColHidden { sheet, col, hidden })
                .collect::<Vec<_>>();
            self::apply_batch(state, sheet, updates)
        })
    }

    /// The row twin of [`App::set_col_hidden`].
    pub fn set_row_hidden(&self, sheet: usize, rows: Range<u32>, hidden: bool) -> Result<usize> {
        if rows.len() as u64 > u64::from(MAX_TRACK_RUN) {
            return Err(Error::TooLarge(rows.len() as u64));
        }
        self.mutate(|state| {
            if state.doc.sheet(sheet).is_none() {
                return Err(Error::NoSuchSheet(sheet));
            }
            let updates = rows
                .filter(|row| {
                    state
                        .doc
                        .sheet(sheet)
                        .is_some_and(|s| s.row_manually_hidden(*row) != hidden)
                })
                .map(|row| Action::SetRowHidden { sheet, row, hidden })
                .collect::<Vec<_>>();
            self::apply_batch(state, sheet, updates)
        })
    }

    /// Every column hidden by hand, in order.
    pub fn hidden_cols(&self, sheet: usize) -> Result<Vec<u32>> {
        let state = self.state.read().unwrap();
        let s = state.doc.sheet(sheet).ok_or(Error::NoSuchSheet(sheet))?;
        Ok(s.hidden_cols().collect())
    }

    /// Every row hidden by hand — unlike [`App::hidden_rows`], which is what the filter hides.
    pub fn manually_hidden_rows(&self, sheet: usize) -> Result<Vec<u32>> {
        let state = self.state.read().unwrap();
        let s = state.doc.sheet(sheet).ok_or(Error::NoSuchSheet(sheet))?;
        Ok(s.manually_hidden_rows().collect())
    }

    /// A track size as it will be stored: bounded, and a length something can measure.
    fn track_size(&self, tracks: u64, size: Option<String>) -> Result<Option<String>> {
        if tracks > u64::from(MAX_TRACK_RUN) {
            return Err(Error::TooLarge(tracks));
        }
        match size {
            Some(size) => match style::length_mm(&size) {
                Some(mm) if mm > 0.0 => Ok(Some(size)),
                _ => Err(Error::Formula(format!("not a positive ODF length: {size}"))),
            },
            None => Ok(None),
        }
    }

    /// Every sized column of a sheet, in order, for a renderer's prefix sums: offsets count from
    /// column zero, so the whole sheet rather than a viewport. Sparse and capped by
    /// [`MAX_TRACK_RUN`].
    pub fn col_widths(&self, sheet: usize) -> Result<Vec<(u32, String)>> {
        let state = self.state.read().unwrap();
        let sheet = state.doc.sheet(sheet).ok_or(Error::NoSuchSheet(sheet))?;
        Ok(sheet.col_widths().map(|(c, w)| (c, w.to_owned())).collect())
    }

    /// The rows of [`App::row_heights`] whose height measures their content, which a grid lets
    /// them outgrow ([`crate::autoheight::heights`]).
    pub fn measured_rows(&self, sheet: usize) -> Result<std::collections::BTreeSet<u32>> {
        let state = self.state.read().unwrap();
        let sheet = state.doc.sheet(sheet).ok_or(Error::NoSuchSheet(sheet))?;
        Ok(sheet.measured_rows().collect())
    }

    /// The row twin of [`App::col_widths`].
    pub fn row_heights(&self, sheet: usize) -> Result<Vec<(u32, String)>> {
        let state = self.state.read().unwrap();
        let sheet = state.doc.sheet(sheet).ok_or(Error::NoSuchSheet(sheet))?;
        Ok(sheet
            .row_heights()
            .map(|(r, h)| (r, h.to_owned()))
            .collect())
    }
}
