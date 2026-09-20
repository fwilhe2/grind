// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Markup Compatibility and Extensibility (ECMA-376 Part 3) — the tax real files charge.
//!
//! Every file written after about 2010 uses it, because it is how a producer writes content
//! a consumer may not understand. Measured on LibreOffice's own corpus: 15 of 360 workbooks
//! carry an `mc:AlternateContent` in a worksheet, and that corpus is minimal bug
//! reproductions rather than documents somebody worked in (`doc/xlsx-format.md` §1.3).
//!
//! Three constructs, and only one of them needs code:
//!
//! - **`mc:Ignorable="x14ac xr"`** names prefixes whose *attributes* may be skipped. Nothing
//!   to do: an attribute in a namespace no table knows already misses every lookup. It is
//!   named here so that nobody builds something.
//! - **`mc:AlternateContent`** is the one. See [`Alternative`].
//! - **`mc:MustUnderstand`** is a producer asserting a consumer cannot proceed. It is
//!   reported and **never refused** — in a spreadsheet it guards a feature rather than the
//!   cell values, and cell values are what an import is for.
//!
//! Everything here is a pure function over names and attribute strings, so all of it is
//! tested without an XML document in sight. The *placement* — once, in the element
//! dispatcher rather than in each context that might contain one — is `xml.rs`'s job, and it
//! matters: `mc:AlternateContent` can appear around a sparkline group, a slicer, a data
//! validation, a drawing or a `sheetPr`, so per-site handling is per-site bugs.

/// What an element in the markup-compatibility namespace is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Alternative {
    /// `<mc:AlternateContent>` — a wrapper holding the alternatives below.
    Wrapper,
    /// `<mc:Choice Requires="x14">` — content for a consumer that understands `x14`.
    Choice,
    /// `<mc:Fallback>` — content for one that understands none of the choices.
    Fallback,
    /// Something else in that namespace. Ignored like any other unknown element.
    Other,
}

pub fn classify(local: &str) -> Alternative {
    match local {
        "AlternateContent" => Alternative::Wrapper,
        "Choice" => Alternative::Choice,
        "Fallback" => Alternative::Fallback,
        _ => Alternative::Other,
    }
}

/// Do we satisfy a `Requires="…"`?
///
/// **This filter understands no extension namespace at all**, so the answer is yes only for
/// the empty requirement — which is to say, effectively never, and the `mc:Fallback` is what
/// gets read. That is the correct answer rather than a limitation: a `Choice` requiring
/// `x14` holds `x14` content, and reading it would mean modelling `x14`.
///
/// Written as a function rather than as `false` so that the day one extension namespace *is*
/// understood, there is one place to say so and one test to extend.
pub fn satisfies(requires: &str) -> bool {
    requires.split_whitespace().next().is_none()
}

/// The namespace prefixes a `mc:MustUnderstand="x14 x15"` names, in order.
///
/// Prefixes, because that is what the attribute holds. The *report* carries URIs: `xml.rs`
/// resolves each one while the declaring element's scope is still live, which is the only
/// moment it can be done (`xml::uri_of`).
pub fn must_understand(value: &str) -> impl Iterator<Item = &str> {
    value.split_whitespace()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_three_elements_that_matter() {
        assert_eq!(classify("AlternateContent"), Alternative::Wrapper);
        assert_eq!(classify("Choice"), Alternative::Choice);
        assert_eq!(classify("Fallback"), Alternative::Fallback);
        assert_eq!(classify("Ignorable"), Alternative::Other);
    }

    /// The rule, stated as a test so that changing it is deliberate.
    #[test]
    fn we_require_nothing_so_we_understand_nothing() {
        assert!(!satisfies("x14"));
        assert!(!satisfies("x14 x15"));
        // Whitespace-only is an empty requirement, not a requirement for a namespace named
        // with a space.
        assert!(satisfies(""));
        assert!(satisfies("   "));
    }

    #[test]
    fn must_understand_is_a_list_of_prefixes() {
        let got: Vec<_> = must_understand("x14  x15\nxr").collect();
        assert_eq!(got, ["x14", "x15", "xr"]);
        assert_eq!(must_understand("").count(), 0);
    }
}
