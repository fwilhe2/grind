// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Zoom — View ▸ Zoom In (⌘+), Zoom Out (⌘−) and Actual Size (⌘0), the platform's keys, and a
//! pinch, which `NSScrollView`'s magnification answers with nothing here. Portable: where a step
//! goes from where the view is. Nothing measured is ever stored zoomed, which is
//! `ui_sheet_gtk`'s M10 rule — magnification is the scroll view's, outside the document.

/// The stops ⌘+ and ⌘− walk, the ones every Mac application with a zoom menu offers near them.
pub const STOPS: [f64; 9] = [0.5, 0.75, 0.9, 1.0, 1.1, 1.25, 1.5, 2.0, 3.0];

/// The magnification a step from `current` lands on: the next stop up (`step > 0`), the next
/// down, or actual size for zero. A pinch may leave the view between stops; a step goes to the
/// nearest one beyond it rather than by a fixed amount.
pub fn stepped(current: f64, step: i8) -> f64 {
    match step {
        0 => 1.0,
        up if up > 0 => STOPS
            .into_iter()
            .find(|stop| *stop > current + 1e-6)
            .unwrap_or(STOPS[STOPS.len() - 1]),
        _ => STOPS
            .into_iter()
            .rev()
            .find(|stop| *stop < current - 1e-6)
            .unwrap_or(STOPS[0]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_step_goes_to_the_next_stop_and_stops_at_the_ends() {
        assert_eq!(stepped(1.0, 1), 1.1);
        assert_eq!(stepped(1.0, -1), 0.9);
        assert_eq!(
            stepped(1.17, 1),
            1.25,
            "from between two stops, the one beyond"
        );
        assert_eq!(stepped(1.17, -1), 1.1);
        assert_eq!(stepped(3.0, 1), 3.0);
        assert_eq!(stepped(0.5, -1), 0.5);
        assert_eq!(stepped(2.0, 0), 1.0, "actual size");
    }
}
