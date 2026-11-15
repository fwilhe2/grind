// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The page view's numbers, in points. **No AppKit**.

/// How wide the caret is.
pub const CARET_W: f64 = 1.5;

/// How tall the page view is for pages `height` tall: never less than the view it fills.
pub fn page_height(height: f64, visible: f64) -> f64 {
    height.max(visible)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_page_fills_its_view() {
        assert_eq!(page_height(100.0, 600.0), 600.0);
        assert_eq!(page_height(1000.0, 600.0), 1000.0);
    }
}
