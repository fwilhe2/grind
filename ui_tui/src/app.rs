// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! What both shells share, which is almost nothing — and that is the point.
//!
//! A spreadsheet's grid and a document's flow have no rendering in common, so there is no
//! shared widget here and no attempt to invent one (`doc/suite.md` rejects a generic
//! `App<D: Document>` for the same reason). What they do share is the *reactive contract*:
//! the core pushes, a shell never polls.

use std::sync::atomic::{AtomicBool, Ordering};

use grind_core::Observer;

/// Set by the core whenever the document changes; the event loop redraws when it is. The
/// shell's half of doc/plan.md rule 3.
#[derive(Default)]
pub struct RedrawFlag(AtomicBool);

impl RedrawFlag {
    pub fn take(&self) -> bool {
        self.0.swap(false, Ordering::SeqCst)
    }

    pub fn raise(&self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

impl Observer for RedrawFlag {
    fn changed(&self) {
        self.raise();
    }
}
