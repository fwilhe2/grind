// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Help while a formula is typed — what may be completed, and what the call under the caret wants
//! next. What to offer, which offer is highlighted and whether Escape dismissed the list for this
//! word are `grind_sheet::formula::assist`'s; what is here is how it reads as one line.
//!
//! **A band, not a popup**: a terminal has no second window, and a list drawn over the grid would
//! need a z-order, a dismissal rule and a redraw region for something that lives three keystrokes.
//! One line under the formula line costs a row.
//!
//! **Enter is deliberately not claimed**: it commits the cell, the common way to finish a formula,
//! where an accidental completion would store the wrong thing. Tab accepts, Up/Down step, Escape
//! dismisses this word's list and leaves the edit open.

use grind_sheet::formula::assist as core_assist;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;

pub use grind_sheet::formula::assist::{Assist, MAX_OFFERS};

/// Whether the band has anything to say at all.
pub fn showing(assist: &Assist) -> bool {
    assist.is_offering() || assist.call.is_some()
}

/// The band's content: the offers if there are any, else the signature of the call the caret is
/// in, the argument being typed picked out. `friendly` picks `Sum(Number: …)` or `SUM(…)`; this
/// shell passes `false`, since its formula line shows display syntax and two names for one
/// function on adjacent rows would confuse.
pub fn line(assist: &Assist, friendly: bool) -> Vec<Span<'static>> {
    let muted = Style::default().add_modifier(Modifier::DIM);
    let strong = Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD);
    if assist.is_offering() {
        let mut spans = vec![Span::styled(" \u{25b8} ", muted)];
        for (index, offer) in assist.offers.iter().take(MAX_OFFERS).enumerate() {
            if index > 0 {
                spans.push(Span::styled("  ", muted));
            }
            spans.push(Span::styled(
                offer.name.clone(),
                if index == assist.chosen {
                    strong
                } else {
                    muted
                },
            ));
        }
        if assist.offers.len() > MAX_OFFERS {
            spans.push(Span::styled(
                format!("  +{}", assist.offers.len() - MAX_OFFERS),
                muted,
            ));
        }
        // The key first and the summary after, since the summary is what runs off a narrow
        // window. Only the chosen offer's summary is shown; the other four are names.
        spans.push(Span::styled("   Tab accepts", muted));
        if let Some(offer) = assist.offers.get(assist.chosen)
            && !offer.detail.is_empty()
        {
            spans.push(Span::styled(format!("  \u{00b7}  {}", offer.detail), muted));
        }
        return spans;
    }
    let Some((name, argument)) = &assist.call else {
        return Vec::new();
    };
    let Some((head, parts)) = core_assist::signature_parts(name, friendly) else {
        return Vec::new();
    };
    let emphasised = core_assist::part_for(name, *argument);
    let mut spans = vec![
        Span::styled(" \u{25b8} ", muted),
        Span::styled(head, muted),
        Span::styled("(", muted),
    ];
    for (index, part) in parts.iter().enumerate() {
        if index > 0 {
            spans.push(Span::styled("; ", muted));
        }
        spans.push(Span::styled(
            part.clone(),
            if index == emphasised { strong } else { muted },
        ));
    }
    spans.push(Span::styled(")", muted));
    spans
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_of(spans: &[Span<'static>]) -> String {
        spans.iter().map(|s| s.content.as_ref()).collect()
    }

    fn read(text: &str) -> Assist {
        let mut assist = Assist::default();
        assist.refresh(text, text.len(), &[]);
        assist
    }

    /// With no word to complete, the band shows the signature of the call the caret is in.
    #[test]
    fn the_band_shows_the_signature_of_the_call_the_caret_is_in() {
        let assist = read("=SUM(A1;");
        assert!(!assist.is_offering(), "nothing half-typed after the `;`");
        assert!(text_of(&line(&assist, false)).contains("SUM"));
        assert!(showing(&assist));
        assert!(!showing(&read("plain text")));
    }

    /// `SUM`'s fifth argument is still its repeating `Number…`, so one part is always picked out.
    #[test]
    fn an_argument_past_the_named_ones_is_still_picked_out() {
        let strong = |text: &str| {
            line(&read(text), false)
                .iter()
                .filter(|span| span.style.fg == Some(Color::Cyan))
                .count()
        };
        assert_eq!(strong("=SUM("), 1);
        assert_eq!(strong("=SUM(1;2;3;4;"), 1);
    }

    /// A long list says how much of itself it is not showing rather than running off the window.
    #[test]
    fn a_long_list_is_cut_to_five_and_says_how_many_more() {
        let assist = read("=A");
        assert!(
            assist.offers.len() > MAX_OFFERS,
            "the catalogue has more than five functions starting with A"
        );
        let line = text_of(&line(&assist, false));
        assert!(
            line.contains(&format!("+{}", assist.offers.len() - MAX_OFFERS)),
            "{line}"
        );
        assert!(line.contains("Tab accepts"));
    }
}
