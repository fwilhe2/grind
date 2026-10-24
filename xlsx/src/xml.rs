// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The tolerant walker, `grind_ooxml::xml`, with `name.is("c")` for SpreadsheetML — the namespace
//! of nearly every element this filter reads.

pub use grind_ooxml::xml::*;

use grind_ooxml::names::Ns;

/// `name.is("c")`: is this `<c>` in SpreadsheetML, in either flavour?
pub trait Spreadsheet {
    fn is(&self, local: &str) -> bool;
}

impl Spreadsheet for Name {
    fn is(&self, local: &str) -> bool {
        self.in_ns(Ns::Spreadsheet, local)
    }
}
