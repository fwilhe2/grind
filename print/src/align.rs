// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Where a laid-out line sits across its measure: `fo:text-align` (`doc/odt-format.md` §5c,
//! fact 8). Pure arithmetic over widths, so it is decided here and only drawn in `text.rs`.
//!
//! Here rather than in `grind_core::layout` because only paper aligns today: no screen draws an
//! aligned line, so no caret has to agree with one yet. The day a shell centres a heading this
//! moves into the core, beside the breaker, so the caret and the ink keep agreeing.

/// `fo:text-align`, left to right only (`doc/text-layout.md`, decision 1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Start,
    Center,
    End,
    Justify,
}

impl Align {
    /// ODF's spelling: `start`/`left`, `center`, `end`/`right`, `justify`; anything else, or
    /// nothing, is the start.
    pub fn parse(value: Option<&str>) -> Align {
        match value.map(str::trim) {
            Some("center") => Align::Center,
            Some("end" | "right") => Align::End,
            Some("justify") => Align::Justify,
            _ => Align::Start,
        }
    }
}

/// How one line is placed: shifted right by `offset`, and every interior space widened by
/// `extra`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fit {
    pub offset: f32,
    pub extra: f32,
}

/// Place a line whose text is `content` wide (trailing spaces left out) in a measure `available`
/// wide, with `spaces` spaces between its first and last visible characters; `last` says whether
/// it is the paragraph's last line, which justification leaves alone.
pub fn fit(align: Align, available: f32, content: f32, spaces: usize, last: bool) -> Fit {
    let slack = (available - content).max(0.0);
    let (offset, extra) = match align {
        Align::Start => (0.0, 0.0),
        Align::Center => (slack / 2.0, 0.0),
        Align::End => (slack, 0.0),
        Align::Justify if last || spaces == 0 => (0.0, 0.0),
        Align::Justify => (0.0, slack / spaces as f32),
    };
    Fit { offset, extra }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn odfs_spellings_are_read_and_anything_else_is_the_start() {
        assert_eq!(Align::parse(Some("center")), Align::Center);
        assert_eq!(Align::parse(Some("end")), Align::End);
        assert_eq!(Align::parse(Some("right")), Align::End);
        assert_eq!(Align::parse(Some("justify")), Align::Justify);
        assert_eq!(Align::parse(Some("left")), Align::Start);
        assert_eq!(Align::parse(Some("start")), Align::Start);
        assert_eq!(Align::parse(Some("sideways")), Align::Start);
        assert_eq!(Align::parse(None), Align::Start);
    }

    #[test]
    fn centre_and_end_shift_the_line_and_widen_nothing() {
        assert_eq!(
            fit(Align::Center, 100.0, 40.0, 3, false),
            Fit {
                offset: 30.0,
                extra: 0.0
            }
        );
        assert_eq!(
            fit(Align::End, 100.0, 40.0, 3, true),
            Fit {
                offset: 60.0,
                extra: 0.0
            }
        );
        assert_eq!(
            fit(Align::Start, 100.0, 40.0, 3, false),
            Fit {
                offset: 0.0,
                extra: 0.0
            }
        );
    }

    #[test]
    fn justification_shares_the_slack_between_the_spaces_except_on_the_last_line() {
        assert_eq!(
            fit(Align::Justify, 100.0, 70.0, 3, false),
            Fit {
                offset: 0.0,
                extra: 10.0
            }
        );
        assert_eq!(
            fit(Align::Justify, 100.0, 70.0, 3, true),
            Fit {
                offset: 0.0,
                extra: 0.0
            }
        );
        assert_eq!(
            fit(Align::Justify, 100.0, 70.0, 0, false),
            Fit {
                offset: 0.0,
                extra: 0.0
            },
            "one word"
        );
    }

    #[test]
    fn a_line_wider_than_its_measure_is_never_pulled_left_or_squeezed() {
        assert_eq!(
            fit(Align::Center, 100.0, 140.0, 2, false),
            Fit {
                offset: 0.0,
                extra: 0.0
            }
        );
        assert_eq!(
            fit(Align::Justify, 100.0, 140.0, 2, false),
            Fit {
                offset: 0.0,
                extra: 0.0
            }
        );
    }
}
