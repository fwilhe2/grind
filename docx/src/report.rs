// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! What the conversion carried, and what it did not — `grind_xlsx::report`'s twin, and for the
//! same reason: **the report is part of the output.** A converter that lies about what it
//! dropped is worse than one that refuses, and `--strict` turns any loss into a non-zero exit.

use std::collections::{BTreeMap, BTreeSet};

pub use grind_ooxml::Flavour;

/// A construct the conversion did not carry.
///
/// The admission rule is `grind_xlsx::Dropped`'s: an entry names something the **output** —
/// the ODF document this filter writes, read by `grind_text` — does not express, never
/// something this filter has not got to yet. That second kind is `doc/docx-import.md`'s
/// milestone table.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Dropped {
    /// A comment whose text the document does not have — its id names nothing in
    /// `word/comments.xml`. Every other comment is carried, as an `office:annotation`.
    Comment,
    /// A tracked change around whole paragraphs or table rows. Changes within a paragraph are
    /// carried as ODF's own; these are applied — the inserted paragraph kept, the deleted one
    /// gone — and their history is not.
    TrackedChange,
    /// A text box (`w:txbxContent`): its text is kept, as paragraphs following the one that
    /// anchors it, and where it floated on the page is not.
    TextBox,
    /// A drawing that is not a picture: a shape, a chart, SmartArt, a canvas.
    Drawing,
    /// An equation (OMML, `m:oMath`).
    Equation,
    /// An embedded object (OLE), or an `altChunk` (a piece of HTML or RTF Word merges in).
    EmbeddedObject,
    /// A picture linked to a file or a URL rather than held in the document. **Never fetched.**
    LinkedPicture,
    /// A floating picture's position: it is kept, anchored to its paragraph, but not where on
    /// the page it floated.
    FloatingPosition,
    /// A field other than a page number, a page count or a hyperlink — a date, a reference, a
    /// table of contents. Its last computed text is kept; it no longer updates.
    Field,
    /// A form field (a checkbox, a text input, a drop-down). Its text is kept.
    FormField,
    /// A content control (`w:sdt`) that is a control — a drop-down, a date picker, a checkbox,
    /// placeholder text. What it shows is kept as text; the control is not.
    ContentControl,
    /// A section set in more than one column.
    Columns,
    /// Numbering on a heading (`1.2 Methods`): the heading is kept, its number is not.
    HeadingNumber,
    /// A table style's conditional formatting — banded rows, a bold first row.
    TableStyleCondition,
    /// A paragraph positioned on the page (`w:framePr`). Its text is kept where it stands.
    Frame,
    /// A VBA project in a macro-enabled document. **Never executed.**
    Macro,
    /// The rest of a document whose XML stops being well formed part-way through. What came
    /// before the damage is kept.
    Damaged,
}

impl Dropped {
    /// A plain-English name, for a report a person reads.
    pub fn label(self) -> &'static str {
        match self {
            Dropped::Comment => "comment",
            Dropped::TrackedChange => "tracked change (accepted)",
            Dropped::TextBox => "text box (its text kept in the flow)",
            Dropped::Drawing => "drawing",
            Dropped::Equation => "equation",
            Dropped::EmbeddedObject => "embedded object",
            Dropped::LinkedPicture => "linked picture",
            Dropped::FloatingPosition => "floating picture's position",
            Dropped::Field => "field (kept as its text)",
            Dropped::FormField => "form field (kept as its text)",
            Dropped::ContentControl => "content control (kept as its text)",
            Dropped::Columns => "multi-column section",
            Dropped::HeadingNumber => "heading number",
            Dropped::TableStyleCondition => "table style banding",
            Dropped::Frame => "positioned paragraph",
            Dropped::Macro => "macro",
            Dropped::Damaged => "damaged remainder of the document",
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Report {
    /// Transitional or Strict — a fact about the file, stated rather than branched on.
    pub flavour: Flavour,
    pub paragraphs: usize,
    pub headings: usize,
    pub list_items: usize,
    pub tables: usize,
    pub images: usize,
    /// Footnotes and endnotes.
    pub notes: usize,
    /// Named paragraph and character styles carried.
    pub styles: usize,
    /// Comments carried, as `office:annotation`s.
    pub comments: usize,
    /// Tracked changes carried, as ODF's own tracked changes.
    pub changes: usize,
    pub dropped: BTreeMap<Dropped, usize>,
    /// Namespaces the file said a consumer must understand and this one does not
    /// (`mc:MustUnderstand`). Recorded, never a refusal.
    pub must_understand: BTreeSet<String>,
}

impl Report {
    pub fn drop_one(&mut self, what: Dropped) {
        self.drop_many(what, 1);
    }

    pub fn drop_many(&mut self, what: Dropped, count: usize) {
        if count > 0 {
            *self.dropped.entry(what).or_default() += count;
        }
    }

    /// Did the conversion lose anything at all? `--strict` is this question.
    pub fn lossless(&self) -> bool {
        self.dropped.is_empty()
    }

    /// One sentence for a shell's notice bar: what arrived, and the kinds of thing that did
    /// not, largest first, at most three named — `grind_xlsx::Report::summary`'s shape, so the
    /// two imports read alike.
    pub fn summary(&self) -> String {
        let plural = |n: usize, one: &str| match n {
            1 => format!("1 {one}"),
            n => format!("{n} {one}s"),
        };
        let mut carried = format!(
            "Imported from Word: {}",
            plural(self.paragraphs, "paragraph")
        );
        if self.tables > 0 {
            carried.push_str(&format!(", {}", plural(self.tables, "table")));
        }
        if self.images > 0 {
            carried.push_str(&format!(", {}", plural(self.images, "picture")));
        }
        if self.lossless() {
            return format!("{carried}; nothing was lost.");
        }
        let mut losses: Vec<(usize, &str)> =
            self.dropped.iter().map(|(k, n)| (*n, k.label())).collect();
        losses.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(b.1)));
        let named: Vec<String> = losses
            .iter()
            .take(3)
            .map(|(n, what)| format!("{what} ×{n}"))
            .collect();
        let more = match losses.len().saturating_sub(3) {
            0 => String::new(),
            1 => " and 1 more kind".to_owned(),
            n => format!(" and {n} more kinds"),
        };
        format!("{carried}; not carried: {}{more}.", named.join(", "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_summary_names_the_largest_losses_first() {
        let mut report = Report {
            paragraphs: 40,
            tables: 1,
            ..Report::default()
        };
        assert_eq!(
            report.summary(),
            "Imported from Word: 40 paragraphs, 1 table; nothing was lost."
        );
        report.drop_many(Dropped::Comment, 3);
        report.drop_one(Dropped::Equation);
        report.drop_many(Dropped::TrackedChange, 7);
        report.drop_one(Dropped::Macro);
        assert_eq!(
            report.summary(),
            "Imported from Word: 40 paragraphs, 1 table; not carried: tracked change \
             (accepted) ×7, comment ×3, equation ×1 and 1 more kind."
        );
    }

    #[test]
    fn a_fresh_report_is_lossless_and_a_count_of_zero_is_no_loss() {
        let mut report = Report::default();
        assert!(report.lossless());
        report.drop_many(Dropped::Comment, 0);
        assert!(report.lossless());
    }
}
