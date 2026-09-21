// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! What both shells share, which is almost nothing — and that is the point.
//!
//! A spreadsheet's grid and a document's flow have no rendering in common, so there is no
//! shared widget here and no attempt to invent one (`doc/suite.md` rejects a generic
//! `App<D: Document>` for the same reason). What they do share is the *reactive contract*:
//! the core pushes, a shell never polls.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use grind_core::Observer;

/// Set by the core whenever the document changes; the event loop redraws when it is. The
/// shell's half of doc/plan.md rule 3.
///
/// It also **counts** the changes, which is how a pane knows whether there is anything unsaved.
/// The flag cannot say so on its own — a pane raises it by hand to repaint presentation state
/// — and neither can `App::can_undo`, which is still true after a save, so a shell that read
/// "modified" off it refused `:q` after every `:w`. A pane records [`RedrawFlag::edits`] when it
/// saves, and anything since is unsaved.
#[derive(Default)]
pub struct RedrawFlag {
    raised: AtomicBool,
    edits: AtomicU64,
}

impl RedrawFlag {
    pub fn take(&self) -> bool {
        self.raised.swap(false, Ordering::SeqCst)
    }

    pub fn raise(&self) {
        self.raised.store(true, Ordering::SeqCst);
    }

    /// How many times the document has changed, ever. Only its equality with an earlier reading
    /// means anything.
    pub fn edits(&self) -> u64 {
        self.edits.load(Ordering::SeqCst)
    }
}

impl Observer for RedrawFlag {
    fn changed(&self) {
        self.edits.fetch_add(1, Ordering::SeqCst);
        self.raise();
    }
}
