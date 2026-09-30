// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The cell editor (M4): one `NSTextField` over the active cell, and what its keys mean.
//!
//! The field is AppKit's, and so is everything inside it — the caret, the selection, dead keys,
//! the emoji picker, every input method — because its field editor is the system's text system.
//! What is this shell's is only what `sheet/state.rs` decides about the selectors the field editor
//! hands its delegate: Return, Tab and their shifted twins commit and move, Esc throws the edit
//! away, and an arrow commits in Enter mode and moves the caret in Edit mode.
//!
//! A commit is `grind_sheet::formula::display::to_input` and then `App::enter`, the path every
//! shell's Enter takes: a formula is typed in display syntax and stored in ODF's. **A formula that
//! will not parse is not stored**: the edit stays open, the banner says why, and the caret goes to
//! the problem — a byte offset turned into the UTF-16 units the field counts
//! (`grind_core::utf16`, hoisted in M1 for exactly this).

use grind_core::utf16;
use grind_sheet::RecalcMode;
use grind_sheet::formula::display;
use grind_sheet::nav::{Dir, Motion};
use objc2::rc::Retained;
use objc2::runtime::{ProtocolObject, Sel};
use objc2::{DefinedClass, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSControl, NSControlTextEditingDelegate, NSTextField, NSTextFieldDelegate, NSTextView,
};
use objc2_foundation::{
    NSNotification, NSObject, NSObjectProtocol, NSPoint, NSRange, NSRect, NSSize, NSString,
};

use crate::banner::Action;
use crate::grid_view::Pane;
use crate::keys::GridAction;
use crate::notice;
use crate::sheet::geom::{HEADER_H, HEADER_W};
use crate::sheet::select;
use crate::sheet::state::{self, Mode, Outcome, Seed};

/// The narrowest the editor is, however narrow its column: room to see what is being typed.
const MIN_W: f64 = 90.0;

/// An edit in progress: the field, its delegate (which the field holds weakly), and the mode.
pub struct Edit {
    field: Retained<NSTextField>,
    _delegate: Retained<EditorDelegate>,
    mode: Mode,
}

define_class!(
    /// The cell editor's delegate: every selector its field editor would act on comes here
    /// first, and `sheet/state.rs` decides.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "GrindCellEditor"]
    #[ivars = std::rc::Weak<Pane>]
    pub struct EditorDelegate;

    unsafe impl NSObjectProtocol for EditorDelegate {}

    unsafe impl NSControlTextEditingDelegate for EditorDelegate {
        #[unsafe(method(control:textView:doCommandBySelector:))]
        fn do_command(&self, _control: &NSControl, _text_view: &NSTextView, selector: Sel) -> bool {
            let name = selector.name().to_str().unwrap_or_default();
            self.ivars()
                .upgrade()
                .is_some_and(|pane| pane.edit_command(name))
        }

        #[unsafe(method(controlTextDidChange:))]
        fn did_change(&self, _notification: &NSNotification) {
            if let Some(pane) = self.ivars().upgrade() {
                pane.edit_changed();
            }
        }
    }

    unsafe impl NSTextFieldDelegate for EditorDelegate {}
);

impl Pane {
    /// Whether a cell is being edited.
    pub fn is_editing(&self) -> bool {
        self.edit.borrow().is_some()
    }

    /// Open the editor over the active cell, seeded with typed text (Enter mode) or with the
    /// cell's own content (Edit mode).
    pub fn begin_edit(&self, seed: Seed) {
        if self.is_editing() {
            return;
        }
        let Some(view) = self.grid_view() else { return };
        let mtm = view.mtm();
        let active = self.selection.get().active;
        let text = match &seed {
            Seed::Text(text) => text.clone(),
            Seed::Cell => self
                .app
                .input_text(self.sheet.get(), active)
                .unwrap_or_default(),
        };
        let cell = self.grid.borrow().cell(active.row, active.col);
        let field = NSTextField::textFieldWithString(&NSString::from_str(&text), mtm);
        field.setFrame(NSRect::new(
            NSPoint::new(cell.x + HEADER_W, cell.y + HEADER_H),
            NSSize::new(cell.w.max(MIN_W), cell.h),
        ));
        let delegate: Retained<EditorDelegate> = {
            let this = EditorDelegate::alloc(mtm).set_ivars(self.me());
            // SAFETY: `init` is `NSObject`'s designated initialiser.
            unsafe { msg_send![super(this), init] }
        };
        // SAFETY: the delegate answers the protocol, and the edit keeps it alive for as long as
        // the field is.
        unsafe { field.setDelegate(Some(ProtocolObject::from_ref(&*delegate))) };
        view.addSubview(&field);
        if let Some(window) = view.window() {
            window.makeFirstResponder(Some(&field));
        }
        // The caret after what is there: a field selects its text as it takes focus, and the next
        // key would replace the seed.
        if let Some(editor) = field.currentEditor() {
            editor.setSelectedRange(NSRange::new(utf16::units_before(&text, text.len()), 0));
        }
        *self.edit.borrow_mut() = Some(Edit {
            field,
            _delegate: delegate,
            mode: seed.mode(),
        });
        self.edit_changed();
    }

    /// What a selector from the field editor does. `true` is "handled": the field editor does
    /// nothing more with it.
    fn edit_command(&self, selector: &str) -> bool {
        let Some(mode) = self.edit.borrow().as_ref().map(|edit| edit.mode) else {
            return false;
        };
        match state::editing(mode, selector) {
            Outcome::Passthrough => false,
            Outcome::Commit(dir) => {
                self.commit(dir);
                true
            }
            Outcome::Cancel => {
                self.end_edit();
                true
            }
            Outcome::ToggleMode => {
                if let Some(edit) = self.edit.borrow_mut().as_mut() {
                    edit.mode = edit.mode.toggled();
                }
                true
            }
        }
    }

    /// Store what the editor holds and move `dir` — or, for a formula that will not parse, keep
    /// the edit open with the caret on the problem.
    fn commit(&self, dir: Option<Dir>) {
        let Some(field) = self.edit.borrow().as_ref().map(|edit| edit.field.clone()) else {
            return;
        };
        let text = field.stringValue().to_string();
        let (sheet, active) = (self.sheet.get(), self.selection.get().active);
        // Unchanged is not an edit: nothing is written, and nothing is marked changed.
        if self
            .app
            .input_text(sheet, active)
            .is_ok_and(|before| before == text)
        {
            self.end_edit();
            self.step(dir);
            return;
        }
        let input = match display::to_input(&text) {
            Ok(input) => input,
            Err(error) => {
                self.say(Some((&notice::bad_formula(&error.message), None)));
                if let Some(editor) = field.currentEditor() {
                    editor.setSelectedRange(NSRange::new(utf16::units_before(&text, error.at), 0));
                }
                return;
            }
        };
        match self.app.enter(sheet, active, &input, RecalcMode::Document) {
            Ok(outcome) => {
                self.end_edit();
                match outcome.recalc.filter(|recalc| recalc.spoiled > 0) {
                    Some(recalc) => self.say(Some((
                        &notice::recalc_skipped(recalc.spoiled),
                        Some(Action::RecalculateAnyway),
                    ))),
                    None => self.say(None),
                }
                self.step(dir);
            }
            // The core refused — a cell it cannot hold, a sheet that is gone — and says why in
            // its own words; the edit stays open so nothing typed is lost.
            Err(error) => self.say(Some((&error.to_string(), None))),
        }
    }

    /// Close the editor without storing anything, and give the keyboard back to the grid.
    fn end_edit(&self) {
        if let Some(edit) = self.edit.borrow_mut().take() {
            edit.field.removeFromSuperview();
        }
        self.focus_grid();
        self.edit_changed();
    }

    /// One cell onward after a commit, the way Return and Tab go.
    fn step(&self, dir: Option<Dir>) {
        let Some(dir) = dir else { return };
        let next = select::apply(
            &self.app,
            self.sheet.get(),
            &self.grid.borrow(),
            self.selection.get(),
            GridAction::Move {
                motion: Motion::By(dir),
                extend: false,
            },
            1,
        );
        self.select(next);
    }

    /// The text being edited, or the active cell's own when nothing is — what the formula
    /// read-out follows.
    pub fn edit_text(&self) -> String {
        match self.edit.borrow().as_ref() {
            Some(edit) => edit.field.stringValue().to_string(),
            None => self
                .app
                .input_text(self.sheet.get(), self.selection.get().active)
                .unwrap_or_default(),
        }
    }

    /// Say a sentence in the banner, with a button when it offers one — or hide it.
    pub fn say(&self, said: Option<(&str, Option<Action>)>) {
        if let Some(banner) = self.banner.borrow().as_ref() {
            banner.say(said);
        }
    }

    /// The banner's button: recalculate after all, and say what that did.
    pub fn recalculate_anyway(&self) {
        match self.app.recalc() {
            Ok(recalc) => self.say(Some((
                &notice::recalculated(recalc.changed, recalc.spoiled),
                None,
            ))),
            Err(error) => self.say(Some((&error.to_string(), None))),
        }
    }
}
