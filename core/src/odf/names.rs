// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Namespace URIs and name resolution. **[GENERIC]**
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
    /// LibreOffice's calc extension namespace. Recognised because it carries a legitimate
    /// alias for `office:value-type` (§9).
    Calcext,
    Other,
}

pub const OFFICE: &str = "urn:oasis:names:tc:opendocument:xmlns:office:1.0";
pub const TABLE: &str = "urn:oasis:names:tc:opendocument:xmlns:table:1.0";
pub const TEXT: &str = "urn:oasis:names:tc:opendocument:xmlns:text:1.0";
pub const CALCEXT: &str = "urn:org:documentfoundation:names:experimental:calc:xmlns:calcext:1.0";

impl Ns {
    pub fn from_uri(uri: &[u8]) -> Ns {
        match uri {
            b if b == OFFICE.as_bytes() => Ns::Office,
            b if b == TABLE.as_bytes() => Ns::Table,
            b if b == TEXT.as_bytes() => Ns::Text,
            b if b == CALCEXT.as_bytes() => Ns::Calcext,
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
