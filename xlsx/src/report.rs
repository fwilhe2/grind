// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! What the conversion carried, and what it did not.
//!
//! **The report is part of the output, not an afterthought.** What a conversion dropped is as
//! important as what it carried, and a converter that lies about it is worse than one that
//! refuses. `--strict` turns any loss into a non-zero exit, which is what a pipeline wants.

use std::collections::{BTreeMap, BTreeSet};

use grind_sheet::model::Pos;

pub use crate::names::Flavour;

/// A construct the **model** has no home for.
///
/// An enum rather than strings so the list is finite, greppable and testable, and so that a
/// new kind of loss is a compile-time decision rather than a new string somewhere.
///
/// The admission rule, which is what keeps it honest: an entry here names something a
/// `grind_sheet::Document` *cannot express*, never something this filter has not got to yet.
/// The second kind belongs in `doc/xlsx-import.md`'s milestone table, where it is work rather
/// than a permanent property of a converted document.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Dropped {
    Chart,
    PivotTable,
    ConditionalFormat,
    DataValidation,
    Comment,
    Drawing,
    Macro,
    ArrayFormula,
    StructuredReference,
    ExternalLink,
    SheetLocalName,
    MergedCells,
    RichText,
    /// A sheet whose data was imported and whose hidden-ness was not. The data is the last
    /// thing to throw away silently, so it is carried and the visibility is counted.
    HiddenSheet,
    ThemeColor,
    FontFamily,
    Protection,
}

impl Dropped {
    /// A plain-English name, for a report a person reads.
    pub fn label(self) -> &'static str {
        match self {
            Dropped::Chart => "chart",
            Dropped::PivotTable => "pivot table",
            Dropped::ConditionalFormat => "conditional format",
            Dropped::DataValidation => "data validation",
            Dropped::Comment => "comment",
            Dropped::Drawing => "drawing",
            Dropped::Macro => "macro",
            Dropped::ArrayFormula => "array formula",
            Dropped::StructuredReference => "structured reference",
            Dropped::ExternalLink => "external link",
            Dropped::SheetLocalName => "sheet-local defined name",
            Dropped::MergedCells => "merged range",
            Dropped::RichText => "rich-text formatting within a cell",
            Dropped::HiddenSheet => "hidden sheet (the data was kept)",
            Dropped::ThemeColor => "theme colour",
            Dropped::FontFamily => "font family",
            Dropped::Protection => "protection",
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Report {
    /// Transitional or Strict — a fact about the file, stated rather than branched on.
    pub flavour: Flavour,
    pub sheets: usize,
    pub cells: usize,
    pub formulas: usize,
    /// Constructs the model has no home for, by kind and count.
    pub dropped: BTreeMap<Dropped, usize>,
    /// Functions a carried formula names that this build does not implement. The cached value
    /// is intact; recalculating would replace it with `#NAME?`.
    pub unknown_functions: BTreeSet<String>,
    /// Cells whose formula could not be translated at all — the value was kept.
    pub untranslated: Vec<(usize, Pos)>,
    /// The same losses by **class**: which kind of expression stopped each one.
    ///
    /// [`Report::untranslated`] says where and this says why, which are different questions
    /// and both worth answering — "four formulas lost" is a number, "four structured
    /// references" is something a person can act on. It is also the scoreboard loop A′ prints
    /// over LibreOffice's corpus.
    pub refused: BTreeMap<crate::formula::Refusal, usize>,
    /// Cells carrying a number format of their own.
    pub formatted: usize,
    /// Cells whose number format lost something, by class — counted once per **cell**, the
    /// way [`Report::refused`] counts a formula, because what a person wants to know is how
    /// much of the document displays differently.
    ///
    /// A class that [`crate::numfmt::Unspellable::refuses`] cost the cell its whole format
    /// and it shows its plain value; every other class cost it a piece and no more.
    pub formats_lost: BTreeMap<crate::numfmt::Unspellable, usize>,
    /// Cells carrying a cell style of their own — a font, a fill, a border or an alignment
    /// that differs from the workbook's default.
    pub styled: usize,
    /// What the workbook's look lost, by class: per **cell** for a piece of a cell's style,
    /// per **track** for a zero-size row or column, and once per **sheet** for panes, an
    /// outline or a sheet-wide column width — [`crate::styles::Appearance`] says which is which.
    ///
    /// [`Report::formats_lost`]'s sibling, and kept apart from [`Report::dropped`] for the same
    /// reason that one is: a construct the model has no home for is a different sentence from
    /// a piece of formatting it has no slot for, and a person deciding whether a conversion is
    /// good enough wants to read them separately.
    pub appearance_lost: BTreeMap<crate::styles::Appearance, usize>,
    /// Cells past `sheet::MAX_CELLS`, read and not carried.
    ///
    /// Not a [`Dropped`] kind, because the model can hold these cells perfectly well and
    /// `Dropped`'s admission rule is *what the model cannot express*. It is a bound this filter
    /// chose, and a bound somebody hit is still a loss: [`Report::lossless`] says so.
    pub over_budget: usize,
    /// Namespaces the file said a consumer must understand and this one does not
    /// (`mc:MustUnderstand`). Recorded, never a refusal.
    pub must_understand: BTreeSet<String>,
    /// Defined names carried into `Document::names` (X5).
    pub names: usize,
    /// Workbook-wide defined names whose expression could not be carried, with the class that
    /// stopped each — a union of two ranges, say, or a spelling ODF has no name for. The name
    /// is gone, and every formula that uses it recalculates to `#NAME?`.
    pub names_lost: Vec<(String, crate::formula::Refusal)>,
    /// Sheets whose name had to change, `(as the workbook spelled it, as the document does)`.
    /// Every reference to one was rewritten with it, so nothing *points* anywhere different;
    /// what is lost is the spelling, and a pipeline that finds sheets by name wants to know.
    pub renamed: Vec<(String, String)>,
}

impl Report {
    pub fn drop_one(&mut self, what: Dropped) {
        self.drop_many(what, 1);
    }

    /// Count one piece of the workbook's look that did not come through.
    pub fn lose(&mut self, what: crate::styles::Appearance) {
        *self.appearance_lost.entry(what).or_default() += 1;
    }

    pub fn drop_many(&mut self, what: Dropped, count: usize) {
        if count > 0 {
            *self.dropped.entry(what).or_default() += count;
        }
    }

    /// One sentence for a shell's notice bar (X6): what arrived, and — the half a person has to
    /// decide about — the kinds of thing that did not, largest first, at most three named.
    ///
    /// Written here rather than in each shell so that four shells say the same sentence about
    /// the same import, and the CLI's report stays the long form of it.
    pub fn summary(&self) -> String {
        let plural = |n: usize, one: &str| match n {
            1 => format!("1 {one}"),
            n => format!("{n} {one}s"),
        };
        let carried = format!(
            "Imported from Excel: {}, {}",
            plural(self.sheets, "sheet"),
            plural(self.cells, "cell")
        );
        if self.lossless() {
            return format!("{carried}; nothing was lost.");
        }
        // Every loss, by what a person would call it, counted the way the report counts it.
        let mut losses: Vec<(usize, String)> = Vec::new();
        losses.extend(self.dropped.iter().map(|(k, n)| (*n, k.label().to_owned())));
        losses.extend(
            self.appearance_lost
                .iter()
                .map(|(k, n)| (*n, k.label().to_owned())),
        );
        losses.extend(
            self.formats_lost
                .iter()
                .map(|(k, n)| (*n, format!("{} (format)", k.label()))),
        );
        // A refusal that is also a `Dropped` kind (an array formula, a structured reference,
        // an external link) is already in the list above; the rest are counted here once.
        let formulas: usize = self
            .refused
            .iter()
            .filter(|(class, _)| class.dropped().is_none())
            .map(|(_, n)| n)
            .sum();
        if formulas > 0 {
            losses.push((formulas, "formula kept as its value".to_owned()));
        }
        if !self.names_lost.is_empty() {
            losses.push((self.names_lost.len(), "defined name".to_owned()));
        }
        if !self.renamed.is_empty() {
            losses.push((self.renamed.len(), "sheet renamed".to_owned()));
        }
        if self.over_budget > 0 {
            losses.push((self.over_budget, "cell past the size limit".to_owned()));
        }
        losses.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
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

    /// Did the conversion lose anything at all? `--strict` is this question.
    ///
    /// An untranslated formula counts: the cell kept Excel's value, so the document is not
    /// wrong, but it no longer recalculates and that is a loss a pipeline should hear about.
    /// An *unknown function* does not — the formula came through intact, and whether this
    /// build can evaluate it is a fact about this build rather than about the conversion.
    pub fn lossless(&self) -> bool {
        self.dropped.is_empty()
            && self.untranslated.is_empty()
            && self.formats_lost.is_empty()
            && self.appearance_lost.is_empty()
            && self.names_lost.is_empty()
            && self.renamed.is_empty()
            && self.over_budget == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_summary_names_the_largest_losses_first() {
        let mut report = Report {
            sheets: 2,
            cells: 120,
            ..Report::default()
        };
        assert_eq!(
            report.summary(),
            "Imported from Excel: 2 sheets, 120 cells; nothing was lost."
        );
        report.drop_many(Dropped::MergedCells, 7);
        report.drop_one(Dropped::Chart);
        report.lose(crate::styles::Appearance::Underline);
        report.lose(crate::styles::Appearance::Underline);
        report.renamed.push(("A/B".into(), "A_B".into()));
        assert_eq!(
            report.summary(),
            "Imported from Excel: 2 sheets, 120 cells; not carried: merged range ×7, \
             underline ×2, chart ×1 and 1 more kind."
        );
    }

    #[test]
    fn a_fresh_report_is_lossless() {
        assert!(Report::default().lossless());
    }

    #[test]
    fn dropping_counts_by_kind() {
        let mut report = Report::default();
        report.drop_one(Dropped::Chart);
        report.drop_one(Dropped::Chart);
        report.drop_many(Dropped::MergedCells, 3);
        report.drop_many(Dropped::Comment, 0);
        assert_eq!(report.dropped[&Dropped::Chart], 2);
        assert_eq!(report.dropped[&Dropped::MergedCells], 3);
        assert!(!report.dropped.contains_key(&Dropped::Comment));
        assert!(!report.lossless());
    }

    /// A function this build has no implementation of is not a conversion loss, and saying it
    /// was would make `--strict` fire on documents that converted perfectly.
    #[test]
    fn an_unknown_function_is_not_a_loss() {
        let mut report = Report::default();
        report.unknown_functions.insert("XLOOKUP".into());
        assert!(report.lossless());
    }
}
