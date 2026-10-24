// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Which face a block is set in on screen — how much bigger than the body, bold or italic,
//! monospace or not — decided by kind and style name; `grind_sheet::look`'s twin. A run's direct
//! formatting reaches layout as a [`crate::CharStyle`]; a heading's size is chosen here, once, and
//! reaches layout through the shell's [`crate::Faces`], so drawing and caret motion agree. A
//! printed page takes the document's own paragraph styles instead ([`crate::paragraph`]). The body
//! size and what "monospace" resolves to stay the shell's — GDI has no generic, so the Windows
//! pane names Consolas.

use crate::BlockKind;

/// How much bigger than the body each heading level is, from level 1: six levels, where authoring
/// stops (`doc/text-core.md`), flattening past level four.
pub const HEADING_SCALE: [f64; 6] = [1.8, 1.5, 1.3, 1.15, 1.05, 1.0];

/// `Title` and `Subtitle`, LibreOffice's blank document's two display styles and the only named
/// styles with a face of their own: larger than any heading, since a title sits above the outline.
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

    /// The face of one block. A named style wins over the block's kind: `Title`, `Subtitle` and a
    /// code fence are paragraphs whose only signal is the name. A heading deeper than six is set
    /// as the sixth.
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

    /// Deeper than six is the sixth, and level 0 is the first — never the body.
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
