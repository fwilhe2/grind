// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The tolerant walker (`grind_ooxml::xml`) plus what is this filter's about it: nearly every
//! element read is WordprocessingML's, and nearly every value is a `w:val` **in the `w:`
//! namespace** — unlike SpreadsheetML, whose attributes are unprefixed (`doc/docx-format.md`
//! §1.4). Asking `attrs.plain("val")` of a Word element finds nothing, which is the one mistake
//! every reader of both vocabularies makes once.

pub use grind_ooxml::xml::*;

use grind_ooxml::names::Ns;

/// `name.w("p")`: is this `<w:p>`, in either flavour?
pub trait Word {
    fn w(&self, local: &str) -> bool;
}

impl Word for Name {
    fn w(&self, local: &str) -> bool {
        self.in_ns(Ns::Word, local)
    }
}

/// Word's attribute readers.
pub trait WordAttrs {
    /// `w:local` — a WordprocessingML attribute, which is namespaced.
    fn w(&self, local: &str) -> Option<&str>;
    /// `w:val`.
    fn val(&self) -> Option<&str> {
        self.w("val")
    }
    /// `w:val` as `ST_OnOff` (§17.17.4): **absent means on** — `<w:b/>` is bold — and `0`,
    /// `false` and `off` are the three spellings of off.
    fn on(&self) -> bool {
        !matches!(self.val(), Some("0" | "false" | "off" | "none"))
    }
    /// `w:local` as an integer. Word writes some measures as decimals (`w:w="100.5"` is
    /// invalid and real), so a decimal is truncated rather than refused.
    fn int(&self, local: &str) -> Option<i64> {
        let raw = self.w(local)?.trim();
        raw.parse::<i64>()
            .ok()
            .or_else(|| raw.parse::<f64>().ok().map(|f| f as i64))
    }
    /// `r:id`, `r:embed` — a relationship id.
    fn rel(&self, local: &str) -> Option<&str>;
}

impl WordAttrs for Attrs {
    fn w(&self, local: &str) -> Option<&str> {
        self.get(Ns::Word, local)
    }

    fn rel(&self, local: &str) -> Option<&str> {
        self.get(Ns::Relationships, local)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: &str = grind_ooxml::names::WORD_T;

    fn attrs_of(xml: &str) -> Attrs {
        let mut reader = Reader::new(xml.as_bytes());
        reader.root().unwrap().expect("a root").1
    }

    #[test]
    fn an_absent_val_on_a_toggle_means_on() {
        assert!(attrs_of(&format!(r#"<w:b xmlns:w="{W}"/>"#)).on());
        assert!(attrs_of(&format!(r#"<w:b xmlns:w="{W}" w:val="true"/>"#)).on());
        for off in ["0", "false", "off"] {
            assert!(!attrs_of(&format!(r#"<w:b xmlns:w="{W}" w:val="{off}"/>"#)).on());
        }
    }

    #[test]
    fn a_word_attribute_is_namespaced_and_a_plain_one_is_not_it() {
        let attrs = attrs_of(&format!(r#"<w:sz xmlns:w="{W}" w:val="24" val="99"/>"#));
        assert_eq!(attrs.val(), Some("24"));
        assert_eq!(attrs.plain("val"), Some("99"));
        assert_eq!(attrs.int("val"), Some(24));
    }

    #[test]
    fn a_decimal_measure_is_truncated_rather_than_refused() {
        let attrs = attrs_of(&format!(r#"<w:ind xmlns:w="{W}" w:left="720.6"/>"#));
        assert_eq!(attrs.int("left"), Some(720));
    }
}
