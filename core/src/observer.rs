// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! How a shell hears that a document changed. **\[GENERIC\]**

/// Notified after every change. Implemented by shells; the core calls it and shells never poll.
///
/// Whoever calls this must drop the write lock first: an observer calls straight back in to
/// re-read, and notifying under the lock deadlocks (`grind_sheet`'s
/// `an_observer_may_read_the_app_without_deadlocking`).
pub trait Observer: Send + Sync {
    fn changed(&self);
}

// Why there is no `Editor` trait yet (`doc/suite.md` proposes one over open, save, undo, redo and
// observe): its error type is undecided. Every failure `open_bytes` and `save_bytes` can produce
// is a `grind_core::Error`, but each `App`'s methods return its own crate's `Result`, so the impl
// would narrow them or the signatures would change. An associated `type Error` keeps both honest
// and gives up a uniform `dyn Editor`. Until that is decided, the suite CLI dispatches on
// `crate::kind` in one `match`.
