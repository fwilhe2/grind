// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! A finger on the grid: one drags it, two pinch it. The grid scrolls by whole cells (the
//! viewport is addressed in rows and columns), so a drag is counted in pixels and spent a track
//! at a time as it crosses them, the remainder kept for the next move.

use grind_sheet::Pos;

use super::layout::Tracks;

/// What the fingers on the grid are doing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Gesture {
    /// One finger, last seen at `at`, with `carry` pixels moved but not yet a whole track.
    Pan { at: (f64, f64), carry: (f64, f64) },
    /// Two fingers, `apart` pixels apart when the zoom was `zoom`.
    Pinch { apart: f64, zoom: f64 },
}

/// Whole tracks a stretch of `carry` pixels crosses starting at track `from`, forward when
/// positive, and the pixels left over.
fn crossed(mut carry: f64, from: u32, tracks: &Tracks) -> (i64, f64) {
    let mut at = from;
    let mut moved = 0;
    loop {
        if carry > 0.0 {
            let size = tracks.size(at).max(1.0);
            if carry < size {
                break;
            }
            carry -= size;
            at += 1;
            moved += 1;
        } else {
            let Some(before) = at.checked_sub(1) else {
                return (moved, 0.0);
            };
            let size = tracks.size(before).max(1.0);
            if -carry < size {
                break;
            }
            carry += size;
            at = before;
            moved -= 1;
        }
    }
    (moved, carry)
}

/// A pan's finger moved to `to`: the rows and columns to scroll by from `scroll`, and the pan
/// as it now is. The content follows the finger, so moving it up scrolls down. `zoom` is how big
/// the grid is drawn.
#[must_use]
pub fn pan(
    gesture: Gesture,
    to: (f64, f64),
    scroll: Pos,
    zoom: f64,
    widths: &Tracks,
    heights: &Tracks,
) -> ((i64, i64), Gesture) {
    let Gesture::Pan { at, carry } = gesture else {
        return ((0, 0), gesture);
    };
    let zoom = zoom.max(0.01);
    let x = carry.0 + (at.0 - to.0) / zoom;
    let y = carry.1 + (at.1 - to.1) / zoom;
    let (cols, x) = crossed(x, scroll.col, widths);
    let (rows, y) = crossed(y, scroll.row, heights);
    (
        (rows, cols),
        Gesture::Pan {
            at: to,
            carry: (x, y),
        },
    )
}

/// The zoom two fingers `apart` pixels apart ask for, from a pinch that began elsewhere.
#[must_use]
pub fn pinch(gesture: Gesture, apart: f64) -> Option<f64> {
    match gesture {
        Gesture::Pinch { apart: from, zoom } if from > 0.0 => Some(zoom * apart / from),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tracks(size: f64) -> Tracks {
        Tracks::new(size, [])
    }

    fn start(x: f64, y: f64) -> Gesture {
        Gesture::Pan {
            at: (x, y),
            carry: (0.0, 0.0),
        }
    }

    #[test]
    fn a_drag_up_scrolls_down_by_the_rows_it_crossed_and_keeps_the_rest() {
        let (moved, now) = pan(
            start(100.0, 300.0),
            (100.0, 240.0),
            Pos::new(0, 0),
            1.0,
            &tracks(80.0),
            &tracks(25.0),
        );
        assert_eq!(moved, (2, 0));
        assert_eq!(
            now,
            Gesture::Pan {
                at: (100.0, 240.0),
                carry: (0.0, 10.0)
            }
        );
    }

    #[test]
    fn the_leftover_counts_towards_the_next_move() {
        let (_, now) = pan(
            start(0.0, 100.0),
            (0.0, 90.0),
            Pos::new(5, 0),
            1.0,
            &tracks(80.0),
            &tracks(25.0),
        );
        let (moved, _) = pan(
            now,
            (0.0, 80.0),
            Pos::new(5, 0),
            1.0,
            &tracks(80.0),
            &tracks(25.0),
        );
        assert_eq!(moved, (0, 0), "20 pixels is not yet a 25-pixel row");
        let (moved, _) = pan(
            now,
            (0.0, 70.0),
            Pos::new(5, 0),
            1.0,
            &tracks(80.0),
            &tracks(25.0),
        );
        assert_eq!(moved, (1, 0));
    }

    #[test]
    fn a_drag_down_and_right_goes_back_but_never_past_the_first_cell() {
        let (moved, now) = pan(
            start(0.0, 0.0),
            (200.0, 200.0),
            Pos::new(1, 1),
            1.0,
            &tracks(80.0),
            &tracks(25.0),
        );
        assert_eq!(moved, (-1, -1));
        assert_eq!(
            now,
            Gesture::Pan {
                at: (200.0, 200.0),
                carry: (0.0, 0.0)
            }
        );
    }

    #[test]
    fn zoomed_in_a_finger_crosses_fewer_cells() {
        let (moved, _) = pan(
            start(0.0, 100.0),
            (0.0, 0.0),
            Pos::new(0, 0),
            2.0,
            &tracks(80.0),
            &tracks(25.0),
        );
        assert_eq!(moved, (2, 0));
    }

    #[test]
    fn fingers_twice_as_far_apart_double_the_zoom() {
        let gesture = Gesture::Pinch {
            apart: 100.0,
            zoom: 0.75,
        };
        assert_eq!(pinch(gesture, 200.0), Some(1.5));
        assert_eq!(pinch(start(0.0, 0.0), 200.0), None);
    }
}
