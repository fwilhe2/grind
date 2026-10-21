// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! What Edit ▸ *Ask Any Model for a Formula…* and *Check a Model's Answer…* say in their alerts
//! (`doc/formula-assistant.md` §11) — the portable half, tested on any host; `structure.rs` runs
//! the alerts.

/// The first `lines` lines of `text`, and a note when there are more — what fits in an alert of
/// something that is whole elsewhere (on the pasteboard).
pub fn head(text: &str, lines: usize) -> String {
    let all: Vec<&str> = text.lines().collect();
    match all.len() > lines {
        true => format!(
            "{}\n… {} more lines — the whole prompt is on the pasteboard.",
            all[..lines].join("\n"),
            all.len() - lines
        ),
        false => text.to_owned(),
    }
}

/// A checked answer as an alert's detail: how it was read, then each target cell and its value —
/// the first thirty, and how many more.
pub fn answer(tried: &grind_assist::copy::Tried) -> String {
    let read = match tried.spelling {
        grind_assist::Spelling::Display => "Read as written.",
        grind_assist::Spelling::Excel => "Read as Excel's syntax.",
    };
    let mut lines = vec![read.to_owned(), String::new()];
    for (pos, value) in tried.values.iter().take(30) {
        let shown = if value.is_empty() { "(empty)" } else { value };
        lines.push(format!("{}\t{shown}", grind_sheet::a1::format(None, *pos)));
    }
    if tried.values.len() > 30 {
        lines.push(format!("… and {} more", tried.values.len() - 30));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use grind_sheet::{App, Pos, RecalcMode};

    #[test]
    fn a_long_prompt_is_cut_and_says_where_the_rest_is() {
        let text: String = (1..=50).map(|i| format!("line {i}\n")).collect();
        let shown = head(&text, 40);
        assert!(shown.starts_with("line 1\n"));
        assert!(shown.contains("line 40\n… 10 more lines"), "{shown}");
        assert_eq!(head("short", 40), "short");
    }

    #[test]
    fn an_answer_says_how_it_was_read_and_every_value() {
        let app = App::new();
        app.enter(0, Pos::new(0, 0), "21", RecalcMode::No).unwrap();
        let b1 = Pos::new(0, 1);
        let tried = grind_assist::copy::try_answer(&app, 0, b1, b1, "=SUM(A1,A1)").unwrap();
        assert_eq!(answer(&tried), "Read as Excel's syntax.\n\nB1\t42");
    }
}
