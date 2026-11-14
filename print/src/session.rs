// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! A document kept typeset while it is open, for a window that shows and edits its pages
//! (`doc/page-model.md` PM3). The pages are set when first asked for after a change and held
//! until the next; nothing is stored in the document.

use std::cell::RefCell;
use std::rc::Rc;

use grind_text::App;

use crate::ops::Document;
use crate::{Fonts, Typesetter, text};

/// One open document's pages and the typesetter that set them. A shell tells it of every
/// change ([`Session::changed`], from its observer) and asks for the pages when it draws.
pub struct Session {
    /// Only the faces built into grind, never the machine's: the same pages anywhere.
    bundled: bool,
    /// The typesetter, and the families the document named when its fonts were looked up.
    setter: RefCell<Option<(Vec<String>, Rc<Typesetter>)>>,
    set: RefCell<Option<Rc<Document>>>,
}

impl Session {
    #[must_use]
    pub fn new(bundled: bool) -> Self {
        Session {
            bundled,
            setter: RefCell::new(None),
            set: RefCell::new(None),
        }
    }

    /// The document changed: its pages are set again when next asked for.
    // ponytail: the whole document is set again, with every measurement and every unchanged
    // paragraph's layout kept — 1.6 ms a page. Re-cut from the first changed block when a
    // document of hundreds of pages is typed in (`doc/page-model.md` PM3 items 3 and 4).
    pub fn changed(&self) {
        self.set.replace(None);
    }

    /// The typesetter for `app`'s document: a new one when the document names a family its
    /// fonts were not looked up for.
    pub fn typesetter(&self, app: &App) -> Rc<Typesetter> {
        let named = if self.bundled {
            Vec::new()
        } else {
            crate::families(app)
        };
        if let Some((known, setter)) = self.setter.borrow().as_ref()
            && named.iter().all(|family| known.contains(family))
        {
            return setter.clone();
        }
        let fonts = if self.bundled {
            Fonts::bundled()
        } else {
            crate::fonts_for(app)
        };
        let setter = Rc::new(Typesetter::new(fonts));
        self.setter.replace(Some((named, setter.clone())));
        self.set.replace(None);
        setter
    }

    /// The document's pages as they are now.
    pub fn document(&self, app: &App) -> Rc<Document> {
        let setter = self.typesetter(app);
        if let Some(set) = self.set.borrow().as_ref() {
            return set.clone();
        }
        let set = Rc::new(text::typeset(app, &setter, &text::Options::default()));
        self.set.replace(Some(set.clone()));
        set
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use grind_text::{BlockKind, Caret};

    #[test]
    fn the_pages_are_held_until_the_document_changes() {
        let app = App::new();
        app.insert(0, BlockKind::Paragraph, "one").unwrap();
        let session = Session::new(true);
        let first = session.document(&app);
        assert!(Rc::ptr_eq(&first, &session.document(&app)), "set once");
        app.insert_text(
            Caret {
                block: 0,
                offset: 3,
            },
            " two",
        )
        .unwrap();
        session.changed();
        let second = session.document(&app);
        assert!(!Rc::ptr_eq(&first, &second));
        assert_ne!(first.pages[0].ops, second.pages[0].ops);
        assert!(
            Rc::ptr_eq(&session.typesetter(&app), &session.typesetter(&app)),
            "one typesetter, so what it measured is kept"
        );
    }

    #[test]
    fn an_edit_on_one_page_leaves_the_others_as_they_were_drawn() {
        let app = App::new();
        let paragraph = "All work and no play makes a dull document. ".repeat(40);
        for at in 0..8 {
            app.insert(at, BlockKind::Paragraph, &paragraph).unwrap();
        }
        let session = Session::new(true);
        let before = session.document(&app);
        assert!(before.pages.len() >= 3);
        // The last paragraph grows by a word: the pages before it are what they were.
        let last = app.block_count() - 2;
        app.insert_text(
            Caret {
                block: last,
                offset: 0,
            },
            "Still ",
        )
        .unwrap();
        session.changed();
        let after = session.document(&app);
        assert_eq!(before.pages[0], after.pages[0]);
        assert_ne!(before.pages.last(), after.pages.last());
    }
}
