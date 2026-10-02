// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! How big each track on one axis is — column widths or row heights — in whatever unit a shell
//! draws in.
//!
//! Hoisted out of `ui_sheet_gtk/src/geom.rs` and `ui_win32/src/sheet/geom.rs`, which each
//! carried the same prefix-sum axis, when the macOS shell would have been the third copy
//! (`doc/macos-shell.md`, M1). It is spreadsheet vocabulary rather than toolkit vocabulary: a
//! track, a default size, a hidden track as a size of *zero*, and which track an offset falls
//! in. What unit an offset is in is the one thing a shell decides, which is why
//! [`Sizes::from_lengths`] takes the conversion from millimetres as a parameter — GDI's 96
//! pixels to the inch times a monitor's DPI, Pango's pixels times a zoom, CoreGraphics' 72
//! points to the inch.

/// How big each track on one axis is — column widths, or row heights.
///
/// One type for both, because the arithmetic is the same and a sheet that got its columns right
/// and its rows wrong is the bug two copies would produce. A document sizes a handful of tracks
/// out of sixteen thousand, so the sparse list plus a running total is what makes [`Sizes::at`]
/// a binary search rather than a walk from column A.
#[derive(Clone, Debug, PartialEq)]
pub struct Sizes {
    default: f64,
    count: u32,
    /// Ascending by index, distinct: the tracks the document gave a size of their own.
    sizes: Vec<(u32, f64)>,
    /// `run[k]` is how much the first `k` entries have displaced everything after them — the
    /// sum of their sizes *minus* what the default would have been. An offset is then
    /// `index * default + run[entries before it]`, with no walk over the sheet.
    run: Vec<f64>,
}

impl Sizes {
    /// `sizes` is taken in any order; ties keep the last one given.
    pub fn new(default: f64, count: u32, mut sizes: Vec<(u32, f64)>) -> Self {
        // Reversed first, so that a stable sort leaves the *last* entry given for an index at
        // the front of its run and `dedup` keeps that one. `with` and `from_lengths` rely on it.
        sizes.reverse();
        sizes.sort_by_key(|(i, _)| *i);
        sizes.dedup_by_key(|(i, _)| *i);
        // A zero is kept: that is a hidden track — a row a filter excludes (§9.4) — and it has
        // to displace nothing rather than fall back to the default.
        sizes.retain(|(i, size)| *i < count && *size >= 0.0);
        let mut run = Vec::with_capacity(sizes.len() + 1);
        let mut acc = 0.0;
        run.push(acc);
        for (_, size) in &sizes {
            acc += size - default;
            run.push(acc);
        }
        Self {
            default,
            count,
            sizes,
            run,
        }
    }

    /// The axis a document describes: its sized tracks as ODF lengths, turned into a shell's
    /// unit by `unit`, which is handed millimetres.
    ///
    /// The one place a physical length becomes a device one, so that "columns are as wide as
    /// the document says" is a property of this function rather than of every caller. A length
    /// this build cannot parse falls back to the default width rather than to zero, because
    /// zero means *hidden* and silently hiding a column is worse than mis-sizing one.
    ///
    /// `unit` is a function rather than a factor so that a shell keeps its own order of
    /// arithmetic: `ui_win32` scales by the monitor's DPI *after* converting to 96-dpi pixels,
    /// and a frame that must come out byte-identical cannot have that reassociated under it.
    ///
    /// `hidden` is the tracks the document hides, which is a **separate question from their
    /// size** — ODF hides a track with `table:visibility="collapse"` (§5.4), not by giving it a
    /// width of zero, so a hidden column usually still carries a perfectly ordinary
    /// `style:column-width`. They go on *after* the lengths, so a hidden track is hidden
    /// whatever size it was given: [`Sizes::new`] keeps the last entry for an index.
    pub fn from_lengths(
        default: f64,
        count: u32,
        lengths: &[(u32, String)],
        hidden: &[u32],
        unit: impl Fn(f64) -> f64,
    ) -> Self {
        let mut sizes: Vec<(u32, f64)> = lengths
            .iter()
            .filter_map(|(index, length)| {
                Some((*index, unit(grind_core::style::length_mm(length)?)))
            })
            .collect();
        sizes.extend(hidden.iter().map(|index| (*index, 0.0)));
        Self::new(default, count, sizes)
    }

    /// How many entries lie strictly before `index`.
    fn before(&self, index: u32) -> usize {
        self.sizes.partition_point(|(i, _)| *i < index)
    }

    pub fn size_of(&self, index: u32) -> f64 {
        match self.sizes.binary_search_by_key(&index, |(i, _)| *i) {
            Ok(k) => self.sizes[k].1,
            Err(_) => self.default,
        }
    }

    /// Content-space offset of a track's leading edge. `count` itself is the far end.
    pub fn offset_of(&self, index: u32) -> f64 {
        f64::from(index) * self.default + self.run[self.before(index)]
    }

    /// The track containing a content-space offset, clamped to the sheet.
    pub fn at(&self, offset: f64) -> u32 {
        let offset = offset.max(0.0);
        // The last sized track that starts at or before `offset`; everything else is one of the
        // uniform runs, before it or after it.
        let k = self
            .sizes
            .partition_point(|(i, _)| self.offset_of(*i) <= offset);
        let (from, at) = match k {
            0 => (0, 0.0),
            k => {
                let (i, size) = self.sizes[k - 1];
                let end = self.offset_of(i) + size;
                if offset < end {
                    return i;
                }
                (i + 1, end)
            }
        };
        let index = u64::from(from) + ((offset - at) / self.default) as u64;
        index.min(u64::from(self.count.saturating_sub(1))) as u32
    }

    pub fn count(&self) -> u32 {
        self.count
    }

    /// Content-space size of the whole axis.
    pub fn total(&self) -> f64 {
        self.offset_of(self.count)
    }

    /// The same axis at a zoom factor. Every size scales, including the default, so the
    /// arithmetic above stays in one space and a zoomed grid is not a second layout path.
    pub fn scaled(&self, factor: f64) -> Self {
        Self::new(
            self.default * factor,
            self.count,
            self.sizes
                .iter()
                .map(|(i, size)| (*i, size * factor))
                .collect(),
        )
    }

    /// The same axis with one track resized — what a resize drag paints before it commits.
    pub fn with(&self, index: u32, size: f64) -> Self {
        let mut sizes = self.sizes.clone();
        sizes.push((index, size));
        Self::new(self.default, self.count, sizes)
    }

    /// Whether this track is a hidden one — zero size, kept explicitly rather than falling back
    /// to the default (see [`Sizes::new`]).
    pub fn is_hidden(&self, index: u32) -> bool {
        self.size_of(index) == 0.0
    }

    /// The maximal contiguous run of hidden tracks containing `index`, half-open — or `None`
    /// when `index` is not itself hidden.
    ///
    /// A linear walk in both directions rather than a binary search either side, because a
    /// document hides a handful of tracks at a time (§5.4) and this is only asked about a
    /// track already known to be hidden.
    pub fn hidden_run(&self, index: u32) -> Option<(u32, u32)> {
        if !self.is_hidden(index) {
            return None;
        }
        let mut from = index;
        while from > 0 && self.is_hidden(from - 1) {
            from -= 1;
        }
        let mut to = index + 1;
        while to < self.count && self.is_hidden(to) {
            to += 1;
        }
        Some((from, to))
    }

    /// The first track at or after `from` that has any width, or `None` past the end.
    ///
    /// Scrolling has to skip hidden tracks or the view stops moving: pressing the scrollbar's
    /// arrow lands on a zero-width column, which occupies no pixels, and the screen does not
    /// change while the position number does.
    pub fn next_visible(&self, from: u32) -> Option<u32> {
        (from..self.count).find(|i| !self.is_hidden(*i))
    }

    /// The last track at or before `from` that has any width, or `None` before the start.
    pub fn prev_visible(&self, from: u32) -> Option<u32> {
        (0..=from.min(self.count.saturating_sub(1)))
            .rev()
            .find(|i| !self.is_hidden(*i))
    }

    /// The nearest track to `index` that is not hidden, looking `forward` first and then the
    /// other way.
    ///
    /// What Down-arrow needs. A hidden track occupies no pixels, so a cursor that lands on one
    /// is a cursor nobody can see — the selection is real, the status bar reports it, and the
    /// screen shows nothing at all. Every spreadsheet steps over hidden tracks for exactly this
    /// reason.
    ///
    /// Falls back to `index` itself when every track in both directions is hidden, because
    /// refusing to move is better than moving nowhere in particular.
    pub fn nearest_visible(&self, index: u32, forward: bool) -> u32 {
        if !self.is_hidden(index) {
            return index;
        }
        let (first, second) = match forward {
            true => (self.next_visible(index), self.prev_visible(index)),
            false => (self.prev_visible(index), self.next_visible(index)),
        };
        first.or(second).unwrap_or(index)
    }

    /// The first track that puts `index` at the far end of a `span`-long view — the least
    /// scrolling that brings a track into sight from below or from the right.
    ///
    /// Walked backwards from `index` rather than forwards from the current position, so that
    /// jumping to the far corner of a sheet costs a screenful of arithmetic rather than a
    /// million steps. `index` itself is always the answer's upper bound, which is what makes it
    /// safe to use as one end of a clamp.
    pub fn start_showing(&self, index: u32, span: f64) -> u32 {
        let mut used = self.size_of(index);
        let mut at = index;
        while at > 0 {
            let size = self.size_of(at - 1);
            if used + size > span {
                break;
            }
            used += size;
            at -= 1;
        }
        at
    }

    /// The first track that can sit at the top (or left) of a `span`-long view without leaving
    /// blank space after the last one — a scrollbar's maximum position.
    ///
    /// Answered by walking back from the end rather than by dividing, because the tracks near
    /// the end may be any size at all.
    pub fn last_start(&self, span: f64) -> u32 {
        let mut used = 0.0;
        let mut index = self.count;
        while index > 0 {
            let size = self.size_of(index - 1);
            if used + size > span && used > 0.0 {
                break;
            }
            used += size;
            index -= 1;
        }
        index.min(self.count.saturating_sub(1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MAX_COLS;

    /// 96-dpi pixels to a millimetre — the unit `ui_win32` hands in at 100% scaling.
    const PX_PER_MM: f64 = 96.0 / 25.4;

    /// The sparse case, which is every real document: `offset_of` and `at` are inverses across
    /// the sized tracks *and* the uniform runs either side of them, and the total is the default
    /// everywhere plus what the overrides changed.
    #[test]
    fn sized_tracks_displace_the_ones_after_them_and_nothing_else() {
        // B is narrow, D is wide; A, C and everything past E is the 80px default.
        let s = Sizes::new(80.0, 100, vec![(3, 200.0), (1, 20.0)]);
        assert_eq!(s.size_of(0), 80.0);
        assert_eq!(s.size_of(1), 20.0);
        assert_eq!(s.size_of(3), 200.0);
        assert_eq!(s.offset_of(0), 0.0);
        assert_eq!(s.offset_of(1), 80.0);
        assert_eq!(s.offset_of(2), 100.0);
        assert_eq!(s.offset_of(3), 180.0);
        assert_eq!(s.offset_of(4), 380.0);
        assert_eq!(s.offset_of(5), 460.0);
        assert_eq!(s.total(), 100.0 * 80.0 - 60.0 + 120.0);

        for i in 0..12u32 {
            let (start, end) = (s.offset_of(i), s.offset_of(i + 1));
            assert_eq!(s.at(start), i, "start of {i}");
            assert_eq!(s.at(end - 0.5), i, "end of {i}");
        }
        // Past the last track, not one past it.
        assert_eq!(s.at(1e12), 99);
    }

    #[test]
    fn every_offset_lands_back_in_its_own_track() {
        let s = Sizes::new(80.0, 100, vec![(3, 200.0), (1, 20.0), (7, 0.0)]);
        for i in 0..100u32 {
            if s.is_hidden(i) {
                continue;
            }
            let start = s.offset_of(i);
            assert_eq!(s.at(start), i, "start of {i}");
            assert_eq!(s.at(start + s.size_of(i) - 0.5), i, "end of {i}");
        }
    }

    /// A resize drag repaints through `with`, so setting a track that already has a size has to
    /// replace it rather than pile up beside it.
    #[test]
    fn resizing_the_same_track_twice_keeps_the_last_size() {
        let s = Sizes::new(80.0, 10, Vec::new()).with(2, 30.0).with(2, 50.0);
        assert_eq!(s.size_of(2), 50.0);
        assert_eq!(s.total(), 10.0 * 80.0 - 30.0);
    }

    /// Zooming moves every edge by the factor and keeps the tracks in the same order — a track
    /// that was twice the default still is.
    #[test]
    fn scaling_an_axis_scales_every_track_and_every_offset() {
        let s = Sizes::new(80.0, 100, vec![(1, 20.0), (3, 200.0)]).scaled(1.5);
        assert_eq!(s.size_of(0), 120.0);
        assert_eq!(s.size_of(1), 30.0);
        assert_eq!(s.offset_of(4), 380.0 * 1.5);
        assert_eq!(
            s.total(),
            Sizes::new(80.0, 100, vec![(1, 20.0), (3, 200.0)]).total() * 1.5
        );
        assert_eq!(s.at(s.offset_of(3) + 1.0), 3);
    }

    /// A single hidden track is its own one-track run, and is not confused with its (visible)
    /// neighbours.
    #[test]
    fn a_hidden_track_is_a_run_of_one() {
        let s = Sizes::new(80.0, 100, vec![(2, 0.0)]);
        assert!(!s.is_hidden(1));
        assert!(s.is_hidden(2));
        assert!(!s.is_hidden(3));
        assert_eq!(s.hidden_run(2), Some((2, 3)));
        assert_eq!(s.hidden_run(1), None, "column 1 is not hidden at all");
    }

    /// Several hidden tracks in a row are one run, found the same way from any index inside it.
    #[test]
    fn a_run_of_several_hidden_tracks_is_found_from_any_index_in_it() {
        let s = Sizes::new(80.0, 100, vec![(2, 0.0), (3, 0.0), (4, 0.0)]);
        for i in 2..5 {
            assert_eq!(s.hidden_run(i), Some((2, 5)), "from index {i}");
        }
        assert_eq!(s.hidden_run(1), None);
        assert_eq!(s.hidden_run(5), None);
    }

    /// Column C hidden occupies zero: B and D touch.
    #[test]
    fn hidden_columns_collapse_the_gap_between_their_neighbours() {
        let s = Sizes::new(80.0, 100, vec![(2, 0.0)]);
        assert_eq!(s.offset_of(2), 160.0, "B ends at 160");
        assert_eq!(s.offset_of(3), 160.0, "D starts at the same pixel");
        assert_eq!(s.size_of(2), 0.0);
    }

    /// A column the document sized is that wide in the shell's unit, and the ones after it move
    /// over.
    #[test]
    fn a_documents_own_widths_decide_the_geometry() {
        let lengths = vec![(0, "2.5cm".to_string()), (2, "10mm".to_string())];
        let cols = Sizes::from_lengths(80.0, MAX_COLS, &lengths, &[], |mm| mm * PX_PER_MM);
        assert!((cols.size_of(0) - 25.0 * PX_PER_MM).abs() < 1e-9);
        assert!((cols.size_of(2) - 10.0 * PX_PER_MM).abs() < 1e-9);
        assert_eq!(cols.size_of(1), 80.0, "unsized columns keep the default");
        assert!((cols.offset_of(1) - 25.0 * PX_PER_MM).abs() < 1e-9);
    }

    /// The unit is the shell's: the same inch is 96 pixels on Windows at 100% and 72 points on
    /// a Mac (`doc/macos-shell.md` decision 4).
    #[test]
    fn the_unit_is_whatever_the_shell_says_a_millimetre_is() {
        let lengths = vec![(0, "25.4mm".to_string())];
        let px = Sizes::from_lengths(80.0, MAX_COLS, &lengths, &[], |mm| mm * PX_PER_MM);
        let pt = Sizes::from_lengths(80.0, MAX_COLS, &lengths, &[], |mm| mm * 72.0 / 25.4);
        assert!((px.size_of(0) - 96.0).abs() < 1e-9);
        assert!((pt.size_of(0) - 72.0).abs() < 1e-9);
    }

    /// A length this build cannot parse must not become a hidden column.
    #[test]
    fn an_unreadable_length_falls_back_to_the_default() {
        let lengths = vec![(0, "wide".to_string())];
        let cols = Sizes::from_lengths(80.0, MAX_COLS, &lengths, &[], |mm| mm * PX_PER_MM);
        assert_eq!(cols.size_of(0), 80.0);
    }

    /// A column ODF hides carries a perfectly ordinary width, and `table:visibility` is what
    /// makes it gone. Reading only the widths drew every column of `hidden-rows-cols.fods`.
    #[test]
    fn a_hidden_track_is_hidden_whatever_width_it_was_given() {
        let lengths = vec![(1, "2.5cm".to_string())];
        let cols = Sizes::from_lengths(80.0, MAX_COLS, &lengths, &[1], |mm| mm * PX_PER_MM);
        assert_eq!(cols.size_of(1), 0.0);
        assert!(cols.is_hidden(1));
        // It displaces nothing: column C starts where column B would have.
        assert_eq!(cols.offset_of(2), 80.0);
    }

    /// A cursor on a hidden track is a cursor nobody can see. It steps over, in the direction it
    /// was travelling, and turns round rather than giving up at the ends.
    #[test]
    fn the_nearest_visible_track_is_the_one_in_the_direction_of_travel() {
        let s = Sizes::new(20.0, 10, vec![(2, 0.0), (3, 0.0), (9, 0.0)]);
        assert_eq!(s.nearest_visible(1, true), 1, "not hidden: stay put");
        assert_eq!(s.nearest_visible(2, true), 4, "over both hidden tracks");
        assert_eq!(s.nearest_visible(3, false), 1, "and backwards");
        // At the end there is nothing ahead, so it comes back rather than sitting on nothing.
        assert_eq!(s.nearest_visible(9, true), 8);
        // Every track hidden: stay where you are, because there is nowhere better.
        let none = Sizes::new(20.0, 3, vec![(0, 0.0), (1, 0.0), (2, 0.0)]);
        assert_eq!(none.nearest_visible(1, true), 1);
    }

    /// Scrolling a track into view brings it to the *far* edge and no further, so that arrowing
    /// down one row moves the view by one row rather than centring on it.
    #[test]
    fn the_least_scroll_that_shows_a_track_puts_it_at_the_far_edge() {
        let s = Sizes::new(20.0, 100, vec![]);
        // A 100px view holds five 20px rows, so row 9 at the bottom means row 5 at the top.
        assert_eq!(s.start_showing(9, 100.0), 5);
        // A track already at the top needs no scrolling, and one taller than the view is shown
        // from its own start rather than not at all.
        assert_eq!(s.start_showing(0, 100.0), 0);
        assert_eq!(s.start_showing(3, 5.0), 3);
    }

    /// The scrollbar's far end leaves the last track at the far edge of the view, not a
    /// screenful of blank space after it.
    #[test]
    fn the_last_start_fills_the_view_to_the_last_track() {
        let s = Sizes::new(20.0, 100, vec![]);
        assert_eq!(s.last_start(100.0), 95);
        // A view smaller than one track still has a last track to show.
        assert_eq!(s.last_start(5.0), 99);
        assert_eq!(Sizes::new(20.0, 3, vec![]).last_start(1000.0), 0);
    }
}
