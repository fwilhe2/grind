// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Which face a block is set in — how much bigger than the body, bold or italic, monospace or
//! not. `grind_sheet::look`'s twin for a page.
//!
//! **The face is decided per block, by kind and style name, and not by the document**: a run's
//! *direct* formatting reaches layout as a [`crate::CharStyle`], but a heading's size is a
//! paragraph **style** and style definitions are not read (`doc/text-core.md`). So every shell
//! that draws a page has to choose a face for a heading itself, and a heading laid out in one
//! face and drawn in another would put every caret in the wrong place — which is why the choice
//! reaches layout through the shell's [`crate::Faces`] and must be *one* choice.
//!
//! It was three: the GNOME window, the browser pane and the Windows pane each carried the same
//! six heading scales, and each its own reading of which name wins. The macOS page would have
//! been the fourth (`doc/macos-shell.md`, M6). They had drifted twice, and both are settled here:
//!
//! * the browser set a `Title` at 2.2 times the body where both desktop windows set it at 2.4;
//! * the browser set a heading deeper than six levels in the **body** face, where the other two
//!   set it as the sixth level — which is what all three said they did.
//!
//! What stays a shell's: the body size itself, in its own unit, and which family "monospace"
//! is — a generic that fontconfig and a browser resolve and GDI does not, so the Windows pane
//! names Consolas and the Mac its own.

use crate::BlockKind;

/// How much bigger than the body each heading level is, from level 1.
///
/// Six levels because that is where `doc/text-core.md` stops authoring, and flat after level
/// four because a level-6 heading barely larger than the paragraph under it is exactly what a
/// level-6 heading should look like.
pub const HEADING_SCALE: [f64; 6] = [1.8, 1.5, 1.3, 1.15, 1.05, 1.0];

/// `Title` and `Subtitle`: the two named paragraph styles LibreOffice's own blank document
/// offers, and the only two given a face of their own. Larger than any heading, since a
/// document's title sits above its outline rather than in it. Everything else in
/// `office:styles` is a name this build keeps and does not interpret.
pub const TITLE_SCALE: f64 = 2.4;
pub const SUBTITLE_SCALE: f64 = 1.3;

/// Which of the faces a page is set in a block takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Role {
    Body,
    /// A heading, at a level from 1 to 6 — one deeper is set as the sixth.
    Heading(u8),
    Title,
    Subtitle,
    /// A fenced code block: [`crate::markdown::PREFORMATTED`], at the body's own size.
    Code,
}

impl Role {
    /// Every face a document can be set in, body first — the list a shell builds its fonts for.
    pub const ALL: [Role; 10] = [
        Role::Body,
        Role::Heading(1),
        Role::Heading(2),
        Role::Heading(3),
        Role::Heading(4),
        Role::Heading(5),
        Role::Heading(6),
        Role::Title,
        Role::Subtitle,
        Role::Code,
    ];

    /// The face of one block. **A named style wins over the block's own kind**: `Title` and
    /// `Subtitle` are paragraphs whose only signal is the name, and a code fence is a paragraph
    /// style and nothing else. A heading deeper than six levels is set as the sixth rather than
    /// refused — the reader is tolerant (R5), so a level-9 heading loads, and a shell that
    /// panicked on one would undo that.
    pub fn of(kind: &BlockKind, style: Option<&str>) -> Role {
        match (style, kind) {
            (Some("Title"), _) => Role::Title,
            (Some("Subtitle"), _) => Role::Subtitle,
            (Some(crate::markdown::PREFORMATTED), _) => Role::Code,
            (_, BlockKind::Heading { level }) => {
                Role::Heading((*level).clamp(1, HEADING_SCALE.len() as u32) as u8)
            }
            _ => Role::Body,
        }
    }

    /// How much bigger than the body this face is.
    pub fn scale(self) -> f64 {
        match self {
            Role::Heading(level) => HEADING_SCALE[usize::from(level.clamp(1, 6)) - 1],
            Role::Title => TITLE_SCALE,
            Role::Subtitle => SUBTITLE_SCALE,
            Role::Body | Role::Code => 1.0,
        }
    }

    pub fn bold(self) -> bool {
        matches!(self, Role::Heading(_) | Role::Title)
    }

    pub fn italic(self) -> bool {
        self == Role::Subtitle
    }

    /// Whether this face is the shell's monospace family rather than its body one.
    pub fn mono(self) -> bool {
        self == Role::Code
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_named_style_wins_over_the_kind() {
        let heading = BlockKind::Heading { level: 2 };
        assert_eq!(Role::of(&heading, None), Role::Heading(2));
        assert_eq!(Role::of(&heading, Some("Title")), Role::Title);
        assert_eq!(
            Role::of(&BlockKind::Paragraph, Some("Subtitle")),
            Role::Subtitle
        );
        assert_eq!(
            Role::of(&BlockKind::Paragraph, Some(crate::markdown::PREFORMATTED)),
            Role::Code
        );
        assert_eq!(
            Role::of(&BlockKind::Paragraph, Some("Text_20_body")),
            Role::Body,
            "a style this build does not interpret is the body"
        );
        assert_eq!(
            Role::of(&BlockKind::ListItem { depth: 2 }, None),
            Role::Body
        );
    }

    /// Deeper than six is the sixth, and a heading claiming level 0 is the first — neither is
    /// the body, which is where the browser used to put a level-7 heading.
    #[test]
    fn a_heading_past_the_last_level_is_set_as_the_last() {
        for level in [7, 9, 200] {
            assert_eq!(
                Role::of(&BlockKind::Heading { level }, None),
                Role::Heading(6)
            );
        }
        assert_eq!(
            Role::of(&BlockKind::Heading { level: 0 }, None),
            Role::Heading(1)
        );
        assert!(Role::Heading(6).bold());
    }

    #[test]
    fn every_face_is_listed_once_and_the_faces_are_what_a_reader_expects() {
        for (i, a) in Role::ALL.iter().enumerate() {
            assert!(!Role::ALL[i + 1..].contains(a), "{a:?} twice");
        }
        assert_eq!(Role::Body.scale(), 1.0);
        assert_eq!(Role::Heading(1).scale(), 1.8);
        assert!(Role::Title.scale() > Role::Heading(1).scale());
        assert!(Role::Title.bold() && !Role::Title.italic());
        assert!(Role::Subtitle.italic() && !Role::Subtitle.bold());
        assert!(Role::Code.mono() && Role::Code.scale() == 1.0);
        assert!(!Role::Body.bold() && !Role::Body.italic() && !Role::Body.mono());
    }
}
