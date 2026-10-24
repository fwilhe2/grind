// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Handling a chart on the grid — selecting, moving, resizing from any edge, deleting — the same
//! in every window (`doc/chart-handling.md`).
//!
//! A click selects a chart, and a selected chart wears an accent outline and eight handles for as
//! long as it is selected. Its body moves it, a handle resizes it with the opposite edge held
//! still, Shift on a corner keeps its proportions, arrows nudge it, Delete and Backspace delete
//! it, Escape lets go. Nothing is written during a drag; the release is one `App::reshape_chart`.
//! All arithmetic is in the shell's unit, with no toolkit.

/// A chart's frame in the shell's own unit: its top-left corner and its size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Frame {
    pub fn new(x: f64, y: f64, w: f64, h: f64) -> Self {
        Frame { x, y, w, h }
    }

    pub fn right(&self) -> f64 {
        self.x + self.w
    }

    pub fn bottom(&self) -> f64 {
        self.y + self.h
    }

    pub fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.x && x < self.right() && y >= self.y && y < self.bottom()
    }
}

/// Which part of a chart the pointer has: its body, or one of its eight handles.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Grip {
    /// Anywhere inside the frame that is not a handle — a drag here moves the chart.
    Body,
    North,
    NorthEast,
    East,
    SouthEast,
    South,
    SouthWest,
    West,
    NorthWest,
}

impl Grip {
    /// The eight handles, clockwise from the top-left corner — the order they are drawn in.
    pub const HANDLES: [Grip; 8] = [
        Grip::NorthWest,
        Grip::North,
        Grip::NorthEast,
        Grip::East,
        Grip::SouthEast,
        Grip::South,
        Grip::SouthWest,
        Grip::West,
    ];

    /// Which edges a drag of this grip moves: (left, top, right, bottom).
    fn edges(self) -> (bool, bool, bool, bool) {
        match self {
            Grip::Body => (true, true, true, true),
            Grip::North => (false, true, false, false),
            Grip::NorthEast => (false, true, true, false),
            Grip::East => (false, false, true, false),
            Grip::SouthEast => (false, false, true, true),
            Grip::South => (false, false, false, true),
            Grip::SouthWest => (true, false, false, true),
            Grip::West => (true, false, false, false),
            Grip::NorthWest => (true, true, false, false),
        }
    }

    /// Whether the grip is one of the four corners — the handles Shift keeps proportions on.
    pub fn is_corner(self) -> bool {
        matches!(
            self,
            Grip::NorthEast | Grip::SouthEast | Grip::SouthWest | Grip::NorthWest
        )
    }

    /// The pointer this grip shows, as a CSS cursor name, which each shell maps to its own.
    pub fn cursor(self) -> &'static str {
        match self {
            Grip::Body => "move",
            Grip::North | Grip::South => "ns-resize",
            Grip::East | Grip::West => "ew-resize",
            Grip::NorthEast | Grip::SouthWest => "nesw-resize",
            Grip::NorthWest | Grip::SouthEast => "nwse-resize",
        }
    }

    /// The handle's centre on `frame`.
    fn centre(self, frame: &Frame) -> (f64, f64) {
        let (cx, cy) = (frame.x + frame.w / 2.0, frame.y + frame.h / 2.0);
        match self {
            Grip::Body => (cx, cy),
            Grip::North => (cx, frame.y),
            Grip::NorthEast => (frame.right(), frame.y),
            Grip::East => (frame.right(), cy),
            Grip::SouthEast => (frame.right(), frame.bottom()),
            Grip::South => (cx, frame.bottom()),
            Grip::SouthWest => (frame.x, frame.bottom()),
            Grip::West => (frame.x, cy),
            Grip::NorthWest => (frame.x, frame.y),
        }
    }
}

/// How big a handle is drawn, in CSS pixels at 1×; a shell scales it.
pub const HANDLE: f64 = 9.0;

/// How far past its square a handle still answers the pointer.
pub const HANDLE_SLOP: f64 = 4.0;

/// The smallest a resize makes a chart, in CSS pixels at 1× — large enough that its handles never
/// overlap.
pub const MIN_SIZE: f64 = 32.0;

/// How far an arrow key nudges a selected chart (Shift+arrow: further), in CSS pixels at 1×.
pub const NUDGE: f64 = 2.0;
pub const NUDGE_LARGE: f64 = 20.0;

/// Below this movement, in CSS pixels at 1×, a press and release is a click that selects.
pub const CLICK_SLOP: f64 = 4.0;

/// Every handle's square on a chart at `frame`, `size` across, centred on its edge or corner so
/// half of it sits outside the chart.
pub fn handles(frame: &Frame, size: f64) -> [(Grip, Frame); 8] {
    Grip::HANDLES.map(|grip| {
        let (cx, cy) = grip.centre(frame);
        (
            grip,
            Frame::new(cx - size / 2.0, cy - size / 2.0, size, size),
        )
    })
}

/// What the point `(x, y)` has of a chart at `frame`: a handle when `selected`, its body when
/// inside it, or nothing. `size` and `slop` are [`HANDLE`] and [`HANDLE_SLOP`] in the shell's
/// unit.
pub fn grip_at(
    frame: &Frame,
    selected: bool,
    x: f64,
    y: f64,
    size: f64,
    slop: f64,
) -> Option<Grip> {
    if selected {
        let reach = size / 2.0 + slop;
        // Corners first: where a corner and an edge handle overlap, the corner is more useful.
        let mut order = Grip::HANDLES;
        order.sort_by_key(|grip| !grip.is_corner());
        for grip in order {
            let (cx, cy) = grip.centre(frame);
            if (x - cx).abs() <= reach && (y - cy).abs() <= reach {
                return Some(grip);
            }
        }
    }
    frame.contains(x, y).then_some(Grip::Body)
}

/// Which of several charts the point has, topmost (last in document order) first, with the grip.
/// The `selected` chart is asked first, since its handles reach outside its frame.
pub fn hit(
    frames: &[Option<Frame>],
    selected: Option<usize>,
    x: f64,
    y: f64,
    size: f64,
    slop: f64,
) -> Option<(usize, Grip)> {
    if let Some(index) = selected
        && let Some(Some(frame)) = frames.get(index)
        && let Some(grip) = grip_at(frame, true, x, y, size, slop)
        && grip != Grip::Body
    {
        return Some((index, grip));
    }
    frames.iter().enumerate().rev().find_map(|(index, frame)| {
        let frame = frame.as_ref()?;
        frame.contains(x, y).then_some((index, Grip::Body))
    })
}

/// Where a drag of `grip` by `(dx, dy)` puts a chart that started at `start`. The body moves the
/// frame; a handle moves its own edges, never shrinking the chart below `min` (the dragged edge
/// stops). With `keep_ratio` a corner keeps the proportions, following the axis the pointer moved
/// further along.
pub fn dragged(start: &Frame, grip: Grip, dx: f64, dy: f64, min: f64, keep_ratio: bool) -> Frame {
    if grip == Grip::Body {
        return Frame::new(start.x + dx, start.y + dy, start.w, start.h);
    }
    let (left, top, right, bottom) = grip.edges();
    let mut w = start.w
        + match (left, right) {
            (true, _) => -dx,
            (_, true) => dx,
            _ => 0.0,
        };
    let mut h = start.h
        + match (top, bottom) {
            (true, _) => -dy,
            (_, true) => dy,
            _ => 0.0,
        };
    if keep_ratio && grip.is_corner() && start.w > 0.0 && start.h > 0.0 {
        let ratio = start.w / start.h;
        // The axis the pointer moved further along (relative to the chart's size) leads.
        if (w / start.w - 1.0).abs() >= (h / start.h - 1.0).abs() {
            h = w / ratio;
        } else {
            w = h * ratio;
        }
        if w < min || h < min {
            let scale = (min / w).max(min / h);
            w *= scale;
            h *= scale;
        }
    }
    let w = w.max(min);
    let h = h.max(min);
    let x = match left {
        true => start.right() - w,
        false => start.x,
    };
    let y = match top {
        true => start.bottom() - h,
        false => start.y,
    };
    Frame::new(x, y, w, h)
}

/// Whether a drag of this far is a move at all, or a click that selects.
pub fn is_click(dx: f64, dy: f64, slop: f64) -> bool {
    dx.abs() < slop && dy.abs() < slop
}

/// What a key does to a selected chart — the same keys in every window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Key {
    /// Delete or Backspace: the chart goes, one undo step.
    Delete,
    /// Escape: let go of the chart; the cells have the keyboard again.
    Deselect,
    /// An arrow: move it by this much, in the shell's unit.
    Nudge(f64, f64),
}

/// What an arrow does, `large` with Shift: `(dx, dy)` in CSS pixels at 1× for the shell to scale.
pub fn nudge(dx: i8, dy: i8, large: bool) -> Key {
    let step = match large {
        true => NUDGE_LARGE,
        false => NUDGE,
    };
    Key::Nudge(f64::from(dx) * step, f64::from(dy) * step)
}

/// A frame no further left or up than the sheet's corner, since `svg:x`/`svg:y` are offsets from
/// it.
pub fn kept_on_sheet(frame: Frame) -> Frame {
    Frame::new(frame.x.max(0.0), frame.y.max(0.0), frame.w, frame.h)
}

/// The sentence a deletion says, naming how to get the chart back.
pub fn deleted_sentence(undo_key: &str) -> String {
    format!("Chart deleted — {undo_key} brings it back")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame() -> Frame {
        Frame::new(100.0, 100.0, 200.0, 100.0)
    }

    #[test]
    fn an_unselected_chart_has_only_a_body() {
        let f = frame();
        assert_eq!(grip_at(&f, false, 150.0, 150.0, 9.0, 4.0), Some(Grip::Body));
        // Its corner, just outside, is nothing until it is selected.
        assert_eq!(grip_at(&f, false, 302.0, 202.0, 9.0, 4.0), None);
        assert_eq!(
            grip_at(&f, true, 302.0, 202.0, 9.0, 4.0),
            Some(Grip::SouthEast)
        );
    }

    #[test]
    fn every_handle_is_found_where_it_is_drawn() {
        let f = frame();
        for (grip, square) in handles(&f, 9.0) {
            let (x, y) = (square.x + square.w / 2.0, square.y + square.h / 2.0);
            assert_eq!(grip_at(&f, true, x, y, 9.0, 4.0), Some(grip), "{grip:?}");
            assert!((square.w - 9.0).abs() < 1e-9);
        }
    }

    #[test]
    fn a_handle_resizes_from_its_own_edge_and_holds_the_other() {
        let f = frame();
        let east = dragged(&f, Grip::East, 50.0, 30.0, 32.0, false);
        assert_eq!(
            east,
            Frame::new(100.0, 100.0, 250.0, 100.0),
            "only the width"
        );
        let west = dragged(&f, Grip::West, 50.0, 0.0, 32.0, false);
        assert_eq!(
            west,
            Frame::new(150.0, 100.0, 150.0, 100.0),
            "the right edge held"
        );
        let nw = dragged(&f, Grip::NorthWest, -20.0, -10.0, 32.0, false);
        assert_eq!(nw, Frame::new(80.0, 90.0, 220.0, 110.0));
        let moved = dragged(&f, Grip::Body, -30.0, 5.0, 32.0, false);
        assert_eq!(moved, Frame::new(70.0, 105.0, 200.0, 100.0));
    }

    #[test]
    fn a_resize_stops_at_the_smallest_size_without_moving_the_far_edge() {
        let f = frame();
        let crushed = dragged(&f, Grip::West, 500.0, 0.0, 32.0, false);
        assert_eq!(crushed.w, 32.0);
        assert_eq!(crushed.right(), f.right(), "the right edge never moves");
        let flat = dragged(&f, Grip::South, 0.0, -500.0, 32.0, false);
        assert_eq!((flat.y, flat.h), (100.0, 32.0));
    }

    #[test]
    fn shift_on_a_corner_keeps_the_proportions() {
        let f = frame();
        let wider = dragged(&f, Grip::SouthEast, 100.0, 0.0, 32.0, true);
        assert_eq!((wider.w, wider.h), (300.0, 150.0));
        let nw = dragged(&f, Grip::NorthWest, 0.0, -50.0, 32.0, true);
        assert_eq!((nw.w, nw.h), (300.0, 150.0));
        assert_eq!((nw.right(), nw.bottom()), (f.right(), f.bottom()));
        // An edge has nothing to keep in proportion.
        assert_eq!(dragged(&f, Grip::East, 100.0, 0.0, 32.0, true).h, 100.0);
    }

    #[test]
    fn the_selected_charts_handles_answer_before_any_body() {
        let a = Some(Frame::new(0.0, 0.0, 100.0, 100.0));
        let b = Some(Frame::new(98.0, 0.0, 100.0, 100.0));
        // On the shared edge, b is on top — until a is selected and its handle is there.
        assert_eq!(
            hit(&[a, b], None, 99.0, 50.0, 9.0, 4.0),
            Some((1, Grip::Body))
        );
        assert_eq!(
            hit(&[a, b], Some(0), 99.0, 50.0, 9.0, 4.0),
            Some((0, Grip::East))
        );
        assert_eq!(hit(&[a, None, b], None, 500.0, 50.0, 9.0, 4.0), None);
    }

    #[test]
    fn the_keys_and_the_corner() {
        assert_eq!(nudge(1, 0, false), Key::Nudge(NUDGE, 0.0));
        assert_eq!(nudge(0, -1, true), Key::Nudge(0.0, -NUDGE_LARGE));
        assert_eq!(
            kept_on_sheet(Frame::new(-5.0, 3.0, 10.0, 10.0)),
            Frame::new(0.0, 3.0, 10.0, 10.0)
        );
        assert!(is_click(2.0, -3.0, CLICK_SLOP));
        assert!(!is_click(5.0, 0.0, CLICK_SLOP));
        assert!(deleted_sentence("Ctrl+Z").contains("Ctrl+Z"));
    }

    #[test]
    fn every_grip_has_a_cursor() {
        assert_eq!(Grip::Body.cursor(), "move");
        for grip in Grip::HANDLES {
            assert!(grip.cursor().ends_with("-resize"), "{grip:?}");
        }
    }
}
