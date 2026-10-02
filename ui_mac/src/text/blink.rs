// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The caret's blink — portable, so its one rule is tested on Linux: **the caret is steady while
//! the user is doing something**, and blinks only once they stop. Every key and click wakes it,
//! and it stays lit for one whole period before it first goes out, which is what every Mac text
//! view does and why the caret is never caught invisible just after it moved.
//!
//! The view owns the timer (`page_view.rs`), which ticks at [`PERIOD`] and asks [`Blink::tick`]
//! whether anything changed. A drive never starts that timer, so a drive's snapshot does not
//! depend on when it was taken (decision 9).

use std::time::Duration;

/// How long the caret is lit, and how long it is out — half a second each, AppKit's default
/// for `NSTextInsertionPointBlinkPeriodOn` and `…Off`.
pub const PERIOD: Duration = Duration::from_millis(500);

/// Whether the caret is lit, and when it was last woken.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Blink {
    pub lit: bool,
    woken: Duration,
}

impl Default for Blink {
    fn default() -> Self {
        Blink {
            lit: true,
            woken: Duration::ZERO,
        }
    }
}

impl Blink {
    /// Something happened at `now`: the caret is lit and stays so for a period.
    pub fn wake(&mut self, now: Duration) {
        self.lit = true;
        self.woken = now;
    }

    /// The timer fired at `now`: the caret goes out or comes back, unless it was woken within
    /// the last period. Answers whether it changed, which is whether to draw it again.
    pub fn tick(&mut self, now: Duration) -> bool {
        if now.saturating_sub(self.woken) < PERIOD {
            return false;
        }
        self.lit = !self.lit;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(ms: u64) -> Duration {
        Duration::from_millis(ms)
    }

    #[test]
    fn the_caret_blinks_only_once_the_user_stops() {
        let mut blink = Blink::default();
        blink.wake(ms(1000));
        assert!(!blink.tick(ms(1200)), "just moved: steady");
        assert!(blink.lit);
        assert!(blink.tick(ms(1500)));
        assert!(!blink.lit, "a period later it goes out");
        assert!(blink.tick(ms(2000)));
        assert!(blink.lit, "and comes back");
        blink.tick(ms(2500));
        assert!(!blink.lit);
        blink.wake(ms(2600));
        assert!(blink.lit, "a key brings it back at once");
        assert!(!blink.tick(ms(3000)));
    }
}
