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

use std::ops::Range;

use grind_core::color::Rgb;
use grind_core::utf16;
use grind_sheet::formula::display;
use grind_sheet::nav::{Dir, Motion, Selection};
use grind_sheet::{RecalcMode, a1};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject, Sel};
use objc2::{DefinedClass, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSColor, NSControl, NSControlTextEditingDelegate, NSFont, NSFontAttributeName,
    NSForegroundColorAttributeName, NSTextField, NSTextFieldDelegate, NSTextView,
};
use objc2_foundation::{
    NSAttributedString, NSAttributedStringKey, NSDictionary, NSMutableAttributedString,
    NSNotification, NSObject, NSObjectProtocol, NSPoint, NSRange, NSRect, NSSize, NSString,
};

use crate::banner::Action;
use crate::grid_view::Pane;
use crate::keys::GridAction;
use crate::notice;
use crate::sheet::assist::{self, Assist, Ink, Piece, Reply};
use crate::sheet::geom::{HEADER_H, HEADER_W};
use crate::sheet::select;
use crate::sheet::state::{self, Mode, Outcome, Seed, Where};

/// The narrowest the editor is, however narrow its column: room to see what is being typed.
const MIN_W: f64 = 90.0;

/// How tall the band under the editor is.
const HINT_H: f64 = 20.0;

/// The band's runs as one attributed line: the offer Tab would take and the argument being typed
/// in the accent and bold, separators and the rest quieter, the signature in the label's ink.
fn attributed(pieces: &[Piece]) -> Retained<NSAttributedString> {
    let line = NSMutableAttributedString::new();
    let size = NSFont::smallSystemFontSize();
    for piece in pieces {
        let (color, font) = match piece.ink {
            Ink::Strong => (
                NSColor::controlAccentColor(),
                NSFont::boldSystemFontOfSize(size),
            ),
            Ink::Muted => (
                NSColor::secondaryLabelColor(),
                NSFont::systemFontOfSize(size),
            ),
            Ink::Plain => (NSColor::labelColor(), NSFont::systemFontOfSize(size)),
        };
        let keys: [&NSAttributedStringKey; 2] =
            // SAFETY: both keys are constants AppKit exports.
            unsafe { [NSForegroundColorAttributeName, NSFontAttributeName] };
        let values: [&AnyObject; 2] = [&color, &font];
        let attributes = NSDictionary::from_slices(&keys, &values);
        // SAFETY: a dictionary of AppKit's own attribute keys to a colour and a font.
        let run = unsafe {
            NSAttributedString::new_with_attributes(&NSString::from_str(&piece.text), &attributes)
        };
        line.appendAttributedString(&run);
    }
    line.into_super()
}

/// An edit in progress: the field, its delegate (which the field holds weakly), the mode, and
/// the help shown under it while a formula is typed (M8).
pub struct Edit {
    field: Retained<NSTextField>,
    _delegate: Retained<EditorDelegate>,
    mode: Mode,
    assist: Assist,
    /// One line under the field: the offers for the word being typed, or the signature of the
    /// call the caret is in — `formula::assist::band`'s runs, hidden when there is nothing to say.
    hint: Retained<NSTextField>,
    /// The reference being pointed at, while one is (Point mode).
    pending: Option<Pending>,
    /// The editor is writing into its own field, and the change it reports is not the user's.
    applying: bool,
}

/// A reference written by pointing: where its text is in the field, as bytes, and the cells it
/// names — so the next arrow moves those cells and rewrites that text.
#[derive(Clone, Debug)]
struct Pending {
    span: Range<usize>,
    selection: Selection,
}

/// The pointed cells' outline — the system's orange, so it is never mistaken for the selection's
/// accent, as a pointed range in every spreadsheet is drawn in a colour of its own.
pub const POINTED: Rgb = (0xff, 0x95, 0x00);

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
        fn do_command(&self, _control: &NSControl, text_view: &NSTextView, selector: Sel) -> bool {
            let name = selector.name().to_str().unwrap_or_default();
            self.ivars()
                .upgrade()
                .is_some_and(|pane| pane.edit_command(name, selector, text_view))
        }

        #[unsafe(method(controlTextDidChange:))]
        fn did_change(&self, _notification: &NSNotification) {
            if let Some(pane) = self.ivars().upgrade() {
                pane.typed_over_pointing();
                pane.refresh_assist();
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
        let hint = NSTextField::labelWithString(&NSString::from_str(""), mtm);
        hint.setFrame(NSRect::new(
            NSPoint::new(cell.x + HEADER_W, cell.y + HEADER_H + cell.h + 2.0),
            NSSize::new(cell.w.max(MIN_W), HINT_H),
        ));
        hint.setDrawsBackground(true);
        hint.setBackgroundColor(Some(&NSColor::controlBackgroundColor()));
        hint.setBordered(true);
        hint.setHidden(true);
        view.addSubview(&hint);
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
            assist: Assist::default(),
            hint,
            pending: None,
            applying: false,
        });
        self.refresh_assist();
        self.edit_changed();
    }

    /// What a selector from the field editor does. `true` is "handled": the field editor does
    /// nothing more with it.
    fn edit_command(&self, selector: &str, sel: Sel, text_view: &NSTextView) -> bool {
        let Some((mode, offering, pending, field)) = self.edit.borrow().as_ref().map(|edit| {
            (
                edit.mode,
                edit.assist.is_offering(),
                edit.pending.is_some(),
                edit.field.clone(),
            )
        }) else {
            return false;
        };
        // A list of offers claims Tab, the arrows and Esc before the edit machine sees them.
        if let Some(reply) = assist::on_selector(offering, selector) {
            self.assist_reply(reply);
            return true;
        }
        let text = field.stringValue().to_string();
        let caret = field.currentEditor().map_or(text.len(), |editor| {
            utf16::byte_of(&text, editor.selectedRange().location)
        });
        let at = Where {
            mode,
            text: &text,
            caret,
            pending,
        };
        match state::editing(at, selector) {
            // A caret move is made here rather than after: the field editor would move the caret
            // once this returns, and the signature band would read where it *was*.
            Outcome::Passthrough if selector.starts_with("move") => {
                // SAFETY: a selector the field editor itself just asked about, sent to it with no
                // argument, which every `move…:` action takes as its sender.
                let moved = unsafe { text_view.tryToPerform_with(sel, None) };
                if moved {
                    self.refresh_assist();
                }
                moved
            }
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
            Outcome::Point { motion, extend } => {
                self.point(motion, extend);
                true
            }
        }
    }

    /// Move — or start — the reference being pointed at. The first arrow points one cell away
    /// from the one being edited, which is where the eye already is.
    fn point(&self, motion: Motion, extend: bool) {
        let pending = self
            .edit
            .borrow()
            .as_ref()
            .and_then(|edit| edit.pending.clone());
        let from = pending.as_ref().map_or_else(
            || Selection::at(self.selection.get().active),
            |pending| pending.selection,
        );
        let moved = select::apply(
            &self.app,
            self.sheet.get(),
            &self.grid.borrow(),
            from,
            GridAction::Move { motion, extend },
            1,
        );
        self.set_pending(moved, pending.map(|pending| pending.span));
    }

    /// Write the reference `selection` names into the field — over `span`, the text written for
    /// the last one, or at the caret for the first — through the field editor, so it is one step
    /// of the field's own undo.
    fn set_pending(&self, selection: Selection, span: Option<Range<usize>>) {
        let Some(field) = self.edit.borrow().as_ref().map(|edit| edit.field.clone()) else {
            return;
        };
        let Some(editor) = field.currentEditor() else {
            return;
        };
        let text = field.stringValue().to_string();
        let (start, end) = selection.rect();
        let reference = display::reference_text(&a1::reference(None, start, end));
        let span = span.unwrap_or_else(|| {
            let caret = utf16::byte_of(&text, editor.selectedRange().location);
            caret..caret
        });
        let range = NSRange::new(
            utf16::units_before(&text, span.start),
            utf16::units_before(&text[span.start..], span.end - span.start),
        );
        if let Some(edit) = self.edit.borrow_mut().as_mut() {
            edit.applying = true;
        }
        editor.replaceCharactersInRange_withString(range, &NSString::from_str(&reference));
        let placed = span.start..span.start + reference.len();
        let now = field.stringValue().to_string();
        editor.setSelectedRange(NSRange::new(utf16::units_before(&now, placed.end), 0));
        if let Some(edit) = self.edit.borrow_mut().as_mut() {
            edit.applying = false;
            edit.pending = Some(Pending {
                span: placed,
                selection,
            });
        }
        // The cells pointed at are brought into sight, or pointing below the fold is typing
        // blind; and drawn, outlined in their own colour.
        if let Some(view) = self.grid_view() {
            view.scrollRectToVisible(crate::grid_view::ns_rect(select::reveal(
                &self.grid.borrow(),
                selection,
            )));
            view.setNeedsDisplay(true);
        }
        self.refresh_assist();
        self.edit_changed();
    }

    /// Text typed by the user ends pointing: the reference stays where it was written, and the
    /// next arrow starts a new one only if a reference could go after what was typed.
    pub(crate) fn typed_over_pointing(&self) {
        let ended = match self.edit.borrow_mut().as_mut() {
            Some(edit) if !edit.applying => edit.pending.take().is_some(),
            _ => false,
        };
        if ended && let Some(view) = self.grid_view() {
            view.setNeedsDisplay(true);
        }
    }

    /// The cells being pointed at, for the grid to outline.
    pub fn pointed(&self) -> Option<Selection> {
        self.edit
            .borrow()
            .as_ref()
            .and_then(|edit| edit.pending.as_ref())
            .map(|pending| pending.selection)
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
            edit.hint.removeFromSuperview();
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

    /// Read what the editor holds and where its caret is, and say what to offer or which
    /// argument the caret is in — asked afresh on every change, never stored between them.
    pub fn refresh_assist(&self) {
        let Some(field) = self.edit.borrow().as_ref().map(|edit| edit.field.clone()) else {
            return;
        };
        let text = field.stringValue().to_string();
        let caret = field.currentEditor().map_or(text.len(), |editor| {
            utf16::byte_of(&text, editor.selectedRange().location)
        });
        let names: Vec<String> = self.app.names().into_iter().map(|(name, _)| name).collect();
        if let Some(edit) = self.edit.borrow_mut().as_mut() {
            edit.assist.refresh(&text, caret, &names);
        }
        self.show_hint();
    }

    /// The band under the editor, or none.
    fn show_hint(&self) {
        let edit = self.edit.borrow();
        let Some(edit) = edit.as_ref() else { return };
        let pieces = assist::band(&edit.assist, self.friendly.get());
        if pieces.is_empty() {
            edit.hint.setHidden(true);
            return;
        }
        edit.hint.setAttributedStringValue(&attributed(&pieces));
        let width = edit
            .hint
            .fittingSize()
            .width
            .max(edit.field.frame().size.width);
        let mut frame = edit.hint.frame();
        frame.size.width = width;
        edit.hint.setFrame(frame);
        edit.hint.setHidden(false);
    }

    /// Tab, an arrow or Esc with a list up: take the offer, step through the list, or close it.
    fn assist_reply(&self, reply: Reply) {
        let accepted = {
            let mut edit = self.edit.borrow_mut();
            let Some(edit) = edit.as_mut() else { return };
            match reply {
                Reply::Accept => edit
                    .assist
                    .accept()
                    .map(|taken| (edit.field.clone(), taken)),
                Reply::Step(by) => {
                    edit.assist.step(by);
                    None
                }
                Reply::Dismiss => {
                    edit.assist.dismiss();
                    None
                }
            }
        };
        if let Some((field, (span, with))) = accepted {
            let text = field.stringValue().to_string();
            let (_, caret) = assist::replaced(&text, span.clone(), &with);
            if let Some(editor) = field.currentEditor() {
                // Through the field editor rather than the field's value, so the replacement is
                // one step of the field's own undo.
                let range = NSRange::new(
                    utf16::units_before(&text, span.start),
                    utf16::units_before(&text[span.start..], span.end - span.start),
                );
                editor.replaceCharactersInRange_withString(range, &NSString::from_str(&with));
                let now = field.stringValue().to_string();
                editor.setSelectedRange(NSRange::new(utf16::units_before(&now, caret), 0));
            }
            self.refresh_assist();
            self.edit_changed();
            return;
        }
        self.show_hint();
    }

    /// View ▸ Friendly Formulas: the read-out and the band in plain English, or not.
    pub fn toggle_friendly(&self) {
        self.friendly.set(!self.friendly.get());
        self.show_hint();
        self.edit_changed();
    }

    /// View ▸ Explain Formula's text: the active cell's formula unfolded a call at a time, or why
    /// there is nothing to unfold.
    pub fn explanation(&self) -> String {
        let text = self
            .app
            .input_text(self.sheet.get(), self.selection.get().active)
            .unwrap_or_default();
        if !text.starts_with('=') {
            return "The active cell holds no formula.".to_owned();
        }
        match grind_sheet::formula::friendly::explain(&text) {
            Ok(explained) => explained,
            Err(error) => notice::bad_formula(&error.message),
        }
    }

    /// Insert ▸ Function…'s choice written in: `NAME(` at the caret when a cell is being edited,
    /// or an edit of the active cell begun with `=NAME(` when not.
    pub fn insert_function(&self, at: usize) {
        let editing = self.is_editing();
        let Some(call) = assist::function_insert(at, editing) else {
            return;
        };
        if !editing {
            self.begin_edit(Seed::Text(call));
            return;
        }
        let Some(field) = self.edit.borrow().as_ref().map(|edit| edit.field.clone()) else {
            return;
        };
        if let Some(editor) = field.currentEditor() {
            let range = editor.selectedRange();
            editor.replaceCharactersInRange_withString(range, &NSString::from_str(&call));
            let caret = range.location + call.encode_utf16().count();
            editor.setSelectedRange(NSRange::new(caret, 0));
        }
        self.refresh_assist();
        self.edit_changed();
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
