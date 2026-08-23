// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! How a shell hears that a document changed. **\[GENERIC\]**
//!
//! One trait, shared by every document type, because the contract has nothing in it that is
//! about cells or paragraphs: something changed, come and re-read it.

/// Notified after every change. Implemented by shells; the core calls it, shells never poll
/// (doc/plan.md, rule 3).
///
/// The one hard rule for whoever calls this: **drop the write lock first.** An observer is
/// expected to call straight back in to re-read what changed, and notifying while still
/// holding the lock deadlocks it. `grind_sheet`'s
/// `an_observer_may_read_the_app_without_deadlocking` test exists because of that, and this
/// note is here so the next document type writes one too.
pub trait Observer: Send + Sync {
    fn changed(&self);
}

// ---------------------------------------------------------------------------
// Why there is no `Editor` trait here yet
// ---------------------------------------------------------------------------
//
// `doc/suite.md` proposes one: the lifecycle half of an `App` — open, save, undo, redo,
// observe — factored out so that `grind`'s suite-level verbs are written once and a shell
// hosting both document types (the terminal and the browser both will, R10) holds one object
// rather than a two-armed match.
//
// It is not here because writing it turned up a question the plan had not asked, and there is
// no second implementation yet to answer it: **what does the trait's error type say?**
//
// * `grind_core::Error` is the honest answer today — every failure `open_bytes` and
//   `save_bytes` can actually produce is generic (a zip that will not open, XML that will not
//   parse, no key, the filesystem). None of `grind_sheet::Error`'s own variants — `NoSuchSheet`,
//   `TooLarge`, `BadSheet`, `Formula` — can come out of either. But `App`'s inherent methods are
//   typed in the *sheet's* Result, so the impl would either narrow them or the signatures would
//   have to change, and it is not yet clear which is right.
// * An associated `type Error` keeps both honest and gives up the uniform `dyn Editor` that
//   was half the point.
//
// Neither branch can be chosen well against one implementation. `grind-text`'s `App` is what
// decides it, and this is the same rule the plan already applies to generalising
// `grind_sheet::odf::source`: the second caller reveals the seam, and guessing before it
// arrives is how a shared abstraction ends up as the union of everything it was meant to
// separate. Until then the suite CLI dispatches on `crate::kind` and calls concrete types,
// which is a `match` in exactly one file.
