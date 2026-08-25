// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Namespace URIs and name resolution. **\[GENERIC\]**
//!
//! Dispatch keys on `(namespace-uri, local-name)`, never on the prefix written in the
//! document (doc/ods-format.md §8.1). Prefixes in the wild vary — `office:`, `ns0:`, a
//! default `xmlns` with no prefix at all — and they can be redeclared on any element, so
//! they carry no meaning. An unrecognised URI is not an error; it resolves to [`Ns::Other`]
//! and every lookup against it simply misses, which routes the element down the
//! ignore-path in `context.rs`.

/// A namespace we recognise. Everything else is [`Ns::Other`] — a value, not a failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Ns {
    Office,
    Table,
    Text,
    /// `number:` — the data-style namespace, whose URI says `datastyle` rather than
    /// `number` (the prefix is conventional, the URI is normative — §8.1).
    Number,
    /// `style:` — style elements and their properties (§5.1).
    Style,
    /// `fo:` — the XSL-FO-compatible properties a style is mostly made of: fonts, colours,
    /// borders, alignment.
    Fo,
    /// `svg:` — ODF's SVG-compatible namespace. Carries `svg:font-family`, which is where a
    /// `style:font-face` says which family it actually stands for.
    Svg,
    /// `xlink:` — W3C XLink, not an ODF namespace at all. Carries `xlink:href`, which is how
    /// every document type spells "points at something": a hyperlink's target, an image's
    /// file, an embedded object's path.
    Xlink,
    /// LibreOffice's calc extension namespace. Recognised because it carries a legitimate
    /// alias for `office:value-type` (§9).
    Calcext,
    /// `draw:` — `draw:frame`, `draw:image`, and everything else a drawing shape is made of.
    /// Only the two above are resolved by anything today; the rest route to `Ignore` like any
    /// other unrecognised content.
    Draw,
    /// `chart:` — `chart:chart`, `chart:series`, `chart:axis` and the rest of a chart's own
    /// embedded document (`grind_sheet::chart`, `doc/chart-format.md`).
    Chart,
    Other,
}

pub const OFFICE: &str = "urn:oasis:names:tc:opendocument:xmlns:office:1.0";
pub const TABLE: &str = "urn:oasis:names:tc:opendocument:xmlns:table:1.0";
pub const TEXT: &str = "urn:oasis:names:tc:opendocument:xmlns:text:1.0";
pub const NUMBER: &str = "urn:oasis:names:tc:opendocument:xmlns:datastyle:1.0";
pub const STYLE: &str = "urn:oasis:names:tc:opendocument:xmlns:style:1.0";
pub const FO: &str = "urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0";
pub const SVG: &str = "urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0";
/// W3C, not OASIS — the one namespace here that ODF borrows rather than defines.
pub const XLINK: &str = "http://www.w3.org/1999/xlink";
pub const CALCEXT: &str = "urn:org:documentfoundation:names:experimental:calc:xmlns:calcext:1.0";
pub const DRAW: &str = "urn:oasis:names:tc:opendocument:xmlns:drawing:1.0";
pub const CHART: &str = "urn:oasis:names:tc:opendocument:xmlns:chart:1.0";

impl Ns {
    pub fn from_uri(uri: &str) -> Ns {
        match uri {
            OFFICE => Ns::Office,
            TABLE => Ns::Table,
            TEXT => Ns::Text,
            NUMBER => Ns::Number,
            STYLE => Ns::Style,
            FO => Ns::Fo,
            SVG => Ns::Svg,
            XLINK => Ns::Xlink,
            CALCEXT => Ns::Calcext,
            DRAW => Ns::Draw,
            CHART => Ns::Chart,
            _ => Ns::Other,
        }
    }
}

/// A resolved element or attribute name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Name {
    pub ns: Ns,
    pub local: String,
}

impl Name {
    pub fn new(ns: Ns, local: impl Into<String>) -> Self {
        Self {
            ns,
            local: local.into(),
        }
    }

    pub fn is(&self, ns: Ns, local: &str) -> bool {
        self.ns == ns && self.local == local
    }
}
