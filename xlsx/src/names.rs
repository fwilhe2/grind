// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Namespace URIs, relationship types, and the flavour they say the file is.
//!
//! The `core/src/odf/names.rs` of this filter, and the same two rules: **dispatch on
//! `(namespace-uri, local-name)` and never on the prefix written in the document**, and **an
//! unrecognised URI is a value rather than a failure** — it resolves to [`Ns::Other`], every
//! lookup against it misses, and the element routes down the ignore path in `xml.rs`.
//!
//! What is different here is that every Part 1 namespace comes in **two** spellings.
//! ISO/IEC 29500 defines Transitional and Strict, real files are overwhelmingly Transitional,
//! and the entire difference that reaches a reader is this file (`doc/xlsx-import.md`,
//! "Transitional, and the real world"). Both families resolve to the same [`Ns`], so nothing
//! downstream branches on the flavour; the reader only *records* which it saw, because that
//! is a useful sentence in a report and a useless one in a parser.
//!
//! Every URI below is `MEASURED` — see `doc/xlsx-format.md` §1.1 and §1.2 for the files and
//! the commands.

/// A namespace this filter recognises. Everything else is [`Ns::Other`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Ns {
    /// SpreadsheetML itself — `worksheet`, `sheetData`, `row`, `c`, `f`, `v`.
    Spreadsheet,
    /// The `r:` attribute namespace: `r:id` on a `<sheet>` names a relationship.
    Relationships,
    /// The root namespace of a `_rels/*.rels` part. **Part 2 (OPC), so it does not vary
    /// between the flavours** — measured, not assumed.
    PackageRels,
    /// `[Content_Types].xml`. Part 2 as well, and likewise invariant.
    ContentTypes,
    /// Markup Compatibility and Extensibility — Part 3, invariant, and the reason `mce.rs`
    /// exists.
    Mce,
    /// DrawingML's main namespace, which is where a theme's colour scheme lives
    /// (`xl/theme/theme1.xml`, `<a:clrScheme>`). **Transitional only**: the Strict spelling
    /// has not been measured — no workbook this project holds carries one — so a Strict
    /// theme resolves to [`Ns::Other`], its colours come back unresolved, and the report
    /// counts them rather than this table guessing (`doc/xlsx-format.md` §4.2).
    Drawing,
    /// No namespace at all. Most SpreadsheetML *attributes* are unprefixed (`<sheet name=…
    /// sheetId=…>`), so this is a real answer rather than an error case.
    None,
    Other,
}

/// Which spelling of Part 1 a file uses.
///
/// A fact the report states, never a branch the reader takes. `Mixed` is not hypothetical:
/// `sc/qa/unit/data/xlsx/universal-content-strict.xlsx` carries Strict relationship types
/// beside Transitional ones in the same `_rels/.rels` (`doc/xlsx-format.md` §1.2).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Flavour {
    /// What Excel has written by default since 2007, and what every other producer writes.
    #[default]
    Transitional,
    /// ISO/IEC 29500 Strict. Recognised; not a supported configuration.
    Strict,
    /// Both, in one document.
    Mixed,
}

/// Which family a Part 1 URI belongs to. Collected as the reader resolves names; `Flavour`
/// falls out of the set at the end.
#[derive(Clone, Copy, Debug, Default)]
pub struct Seen {
    pub transitional: bool,
    pub strict: bool,
}

impl Seen {
    pub fn flavour(self) -> Flavour {
        match (self.transitional, self.strict) {
            (true, true) => Flavour::Mixed,
            (false, true) => Flavour::Strict,
            // A file that declared neither is Transitional by default rather than by
            // evidence. It is also a file with no SpreadsheetML in it, which the caller has
            // already refused for a better reason than this one.
            _ => Flavour::Transitional,
        }
    }
}

// ---- Part 1, Transitional (ECMA-376 1st edition spellings) ----

pub const MAIN_T: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
/// Measured from `styles/colors.xlsx`'s theme part, 2026-09-21.
pub const DRAWING_T: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
pub const REL_T: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

// ---- Part 1, Strict (ISO/IEC 29500 Strict) ----

pub const MAIN_S: &str = "http://purl.oclc.org/ooxml/spreadsheetml/main";
pub const REL_S: &str = "http://purl.oclc.org/ooxml/officeDocument/relationships";

// ---- Parts 2 and 3: one spelling each, in both flavours ----

pub const PACKAGE_RELS: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
pub const CONTENT_TYPES: &str = "http://schemas.openxmlformats.org/package/2006/content-types";
pub const MCE: &str = "http://schemas.openxmlformats.org/markup-compatibility/2006";

impl Ns {
    pub fn from_uri(uri: &str) -> Ns {
        match uri {
            MAIN_T | MAIN_S => Ns::Spreadsheet,
            REL_T | REL_S => Ns::Relationships,
            PACKAGE_RELS => Ns::PackageRels,
            CONTENT_TYPES => Ns::ContentTypes,
            MCE => Ns::Mce,
            DRAWING_T => Ns::Drawing,
            "" => Ns::None,
            _ => Ns::Other,
        }
    }
}

/// Which flavour a URI is evidence of, if it is evidence of either.
///
/// Only Part 1 URIs count. The package and markup-compatibility namespaces are the same in
/// both, so seeing one says nothing — and treating it as a vote for Transitional would make
/// every Strict file `Mixed`.
pub fn family(uri: &str) -> Option<Flavour> {
    // `starts_with` rather than equality because a relationship *type* extends the
    // relationships namespace with a segment, and both are evidence of the same family.
    if uri == MAIN_T || uri.starts_with(REL_T) {
        Some(Flavour::Transitional)
    } else if uri == MAIN_S || uri.starts_with(REL_S) {
        Some(Flavour::Strict)
    } else {
        None
    }
}

/// A relationship's `Type`, which is how a part is found. **Never by path convention** —
/// `xl/workbook.xml` is where every producer puts the workbook and nowhere in the spec
/// promises it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RelType {
    OfficeDocument,
    Worksheet,
    ChartSheet,
    SharedStrings,
    Styles,
    Theme,
    /// Recognised so it can be *ignored* cheaply: a calculation chain is Excel's evaluation
    /// order cache and says nothing this filter wants.
    CalcChain,
    Other,
}

impl RelType {
    /// Both families, by stripping the family's own prefix and matching what is left.
    ///
    /// The prefix is checked rather than the last path segment. Matching only the segment
    /// would accept `http://example.invalid/relationships/worksheet`, which is a different
    /// vocabulary wearing a familiar word.
    pub fn from_uri(uri: &str) -> RelType {
        let rest = uri
            .strip_prefix(REL_T)
            .or_else(|| uri.strip_prefix(REL_S))
            .and_then(|rest| rest.strip_prefix('/'));
        match rest {
            Some("officeDocument") => RelType::OfficeDocument,
            Some("worksheet") => RelType::Worksheet,
            Some("chartsheet") => RelType::ChartSheet,
            Some("sharedStrings") => RelType::SharedStrings,
            Some("styles") => RelType::Styles,
            Some("theme") => RelType::Theme,
            Some("calcChain") => RelType::CalcChain,
            _ => RelType::Other,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_families_resolve_to_one_key() {
        assert_eq!(Ns::from_uri(MAIN_T), Ns::Spreadsheet);
        assert_eq!(Ns::from_uri(MAIN_S), Ns::Spreadsheet);
        assert_eq!(Ns::from_uri(REL_T), Ns::Relationships);
        assert_eq!(Ns::from_uri(REL_S), Ns::Relationships);
    }

    #[test]
    fn an_unknown_uri_is_a_value_not_a_failure() {
        assert_eq!(
            Ns::from_uri("http://schemas.microsoft.com/office/spreadsheetml/2014/revision"),
            Ns::Other
        );
        assert_eq!(Ns::from_uri(""), Ns::None);
    }

    /// The whole point of the flavour machinery: only Part 1 votes.
    #[test]
    fn the_invariant_namespaces_are_evidence_of_nothing() {
        assert_eq!(family(MAIN_T), Some(Flavour::Transitional));
        assert_eq!(family(MAIN_S), Some(Flavour::Strict));
        assert_eq!(family(PACKAGE_RELS), None);
        assert_eq!(family(CONTENT_TYPES), None);
        assert_eq!(family(MCE), None);
    }

    #[test]
    fn a_relationship_type_votes_too() {
        assert_eq!(
            family("http://purl.oclc.org/ooxml/officeDocument/relationships/worksheet"),
            Some(Flavour::Strict)
        );
    }

    #[test]
    fn flavour_from_what_was_seen() {
        let both = Seen {
            transitional: true,
            strict: true,
        };
        assert_eq!(both.flavour(), Flavour::Mixed);
        assert_eq!(Seen::default().flavour(), Flavour::Transitional);
        assert_eq!(
            Seen {
                strict: true,
                ..Seen::default()
            }
            .flavour(),
            Flavour::Strict
        );
    }

    #[test]
    fn relationship_types_in_both_spellings() {
        for base in [REL_T, REL_S] {
            assert_eq!(
                RelType::from_uri(&format!("{base}/officeDocument")),
                RelType::OfficeDocument
            );
            assert_eq!(
                RelType::from_uri(&format!("{base}/worksheet")),
                RelType::Worksheet
            );
            assert_eq!(
                RelType::from_uri(&format!("{base}/sharedStrings")),
                RelType::SharedStrings
            );
        }
    }

    /// Measured: `universal-content-strict.xlsx` really carries this, with a lower-case
    /// `officedocument` and a target of its own (`doc/xlsx-format.md` §1.2). It must come
    /// back `Other` — unrecognised, therefore ignored — rather than being guessed at.
    #[test]
    fn a_misspelt_type_is_unrecognised_rather_than_guessed() {
        assert_eq!(
            RelType::from_uri(
                "http://schemas.openxmlformats.org/officedocument/2006/relationships/metadata/core-properties"
            ),
            RelType::Other
        );
    }

    /// Matching the last segment instead of the prefix would accept this.
    #[test]
    fn a_familiar_word_in_a_foreign_vocabulary_is_not_a_relationship_type() {
        assert_eq!(
            RelType::from_uri("http://example.invalid/relationships/worksheet"),
            RelType::Other
        );
    }
}
