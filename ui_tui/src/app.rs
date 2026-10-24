// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! What both halves share, which is almost nothing: a grid and a flow have no rendering in common,
//! so there is no shared widget. What they share is the reactive contract — the core pushes, a
//! shell never polls.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use std::path::PathBuf;

use grind_core::{DocumentKind, Observer};

/// A request from a pane to replace itself — what `:new` and `:open` ask the event loop for, since a
/// pane cannot build its own successor (it may be the other kind of document). The loop swaps
/// the pane and keeps the terminal, which is the in-session "new" and "open" a window has.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Switch {
    /// An empty document of this kind.
    New(DocumentKind),
    /// This file, whose kind was read from its bytes by `crate::sniff`.
    Open(PathBuf, DocumentKind),
    /// The pane with no document in it (`crate::welcome`) — what `:welcome` goes back to.
    Welcome,
}

/// Set by the core whenever the document changes; the event loop redraws when it is (rule 3).
///
/// It also **counts** changes, which is how a pane knows whether anything is unsaved: the flag
/// alone cannot say (a pane raises it by hand to repaint), and `App::can_undo` stays true after a
/// save. A pane records [`RedrawFlag::edits`] when it saves; anything since is unsaved.
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
