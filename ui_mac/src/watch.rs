// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The core's observer, bridged to a window's state, for the grid and the page.
//! `grind_core::Observer` must be `Send + Sync`, since the core does not say which thread a change
//! arrives on, and a window's state holds views, which are the main thread's. So the observer
//! holds only an address and the state is found in a registry the main thread owns; a notification
//! arriving elsewhere finds no registry and changes nothing (rule 3).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};
use std::sync::Arc;

use objc2::MainThreadMarker;

/// Which way a change went — what `NSDocument`'s change count is told, so undoing back to the
/// saved state takes the Edited dot away again rather than counting the undo as one more edit.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Turn {
    #[default]
    Done,
    Undone,
    Redone,
}

/// What a pane tells about each change to its document — the document, which marks itself
/// edited, or not, by it.
pub type OnChange = std::cell::RefCell<Option<Box<dyn Fn(Turn)>>>;

/// Something that re-reads its document when the core says it changed.
pub trait Watched {
    fn document_changed(&self);
}

thread_local! {
    /// What the core may tell about a change, by the address of each.
    static WATCHED: RefCell<HashMap<usize, Weak<dyn Watched>>> = RefCell::new(HashMap::new());
}

struct Changed(usize);

impl grind_core::Observer for Changed {
    fn changed(&self) {
        if MainThreadMarker::new().is_none() {
            return;
        }
        let watched = WATCHED.with(|all| all.borrow().get(&self.0).and_then(Weak::upgrade));
        if let Some(watched) = watched {
            watched.document_changed();
        }
    }
}

/// An observer that tells `watched` about every change from now on — for its `App`'s
/// `set_observer`.
pub fn observer<T: Watched + 'static>(watched: &Rc<T>) -> Arc<dyn grind_core::Observer> {
    let id = Rc::as_ptr(watched) as *const () as usize;
    let weak: Weak<dyn Watched> = Rc::downgrade(watched) as Weak<dyn Watched>;
    WATCHED.with(|all| {
        let mut all = all.borrow_mut();
        // What is gone leaves its address behind; drop those as new ones arrive.
        all.retain(|_, each| each.strong_count() > 0);
        all.insert(id, weak);
    });
    Arc::new(Changed(id))
}
