// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The currency picker: which format a click on `€`, `$` or `£` writes, and which of the three
//! the active cell already wears — now `grind_sheet::format::{currency, currency_chosen}`,
//! hoisted when the Mac wanted the same answers. Re-exported under this shell's own names so its
//! call sites still read.

pub use grind_sheet::format::currency as format_for;
pub use grind_sheet::format::currency_chosen as chosen;
