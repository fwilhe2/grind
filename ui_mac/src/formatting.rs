// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Formatting (M7), on either pane: a Format command, the font panel's `changeFont:` and the
//! colour panel's `changeColor:`.
//!
//! Every decision is portable — `sheet/format.rs` and `text/format.rs`, over the cores' own
//! vocabularies — and this file only reads what those need off a pane, writes what they answer,
//! and turns an `NSFont` or an `NSColor` into the facts they speak in. The font and colour panels
//! are the platform's (decision 3): nothing here draws a picker, and **Format ▸ Font ▸ Show
//! Fonts** and **Show Colors** are the standard selectors, answered by AppKit.

use grind_core::color::Rgb;
use grind_text::BlockKind;
use grind_text::format::{self as text_format, Landed};
use grind_text::style::CharStyle;
use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::{NSColor, NSFont, NSFontManager, NSFontTraitMask};
use objc2_foundation::NSString;

use crate::grid_view::{Pane, rgb};
use crate::menu::Command;
use crate::metrics::BASE_PT;
use crate::page_view::TextPane;
use crate::sheet::format::{self as cells, Active, Write};
use crate::text::face;
use crate::text::format::{self as page, Facts};

/// A colour as a document stores it.
fn hex((r, g, b): Rgb) -> String {
    format!("#{r:02x}{g:02x}{b:02x}")
}

/// What the font panel can say about `font`.
fn facts(manager: &NSFontManager, font: &NSFont) -> Facts {
    let traits = manager.traitsOfFont(font);
    Facts {
        family: font
            .familyName()
            .map_or_else(String::new, |name| name.to_string()),
        size: font.pointSize(),
        bold: traits.contains(NSFontTraitMask::BoldFontMask),
        italic: traits.contains(NSFontTraitMask::ItalicFontMask),
    }
}

/// An `NSFont` for a style the panel is shown: its family when it names one, the system face
/// otherwise, at its size, bold and italic as it is.
fn ns_font(
    manager: &NSFontManager,
    family: Option<&str>,
    size: f64,
    bold: bool,
    italic: bool,
) -> Retained<NSFont> {
    let base = family
        .and_then(|name| NSFont::fontWithName_size(&NSString::from_str(name), size))
        .unwrap_or_else(|| NSFont::systemFontOfSize(size));
    let mut font = base;
    if bold {
        font = manager.convertFont_toHaveTrait(&font, NSFontTraitMask::BoldFontMask);
    }
    if italic {
        font = manager.convertFont_toHaveTrait(&font, NSFontTraitMask::ItalicFontMask);
    }
    font
}

impl Pane {
    /// The active cell, as `sheet/format.rs` reads it.
    fn active(&self) -> Active {
        let (sheet, at) = (self.sheet.get(), self.selection.get().active);
        let shown = self
            .app
            .get_viewport(sheet, at.row..at.row + 1, at.col..at.col + 1)
            .ok()
            .and_then(|view| {
                view.text(at.row, at.col).map(|text| {
                    grind_sheet::format::decimals_shown(text, self.app.locale().as_ref())
                })
            })
            .unwrap_or(0);
        Active {
            style: self
                .app
                .style_at(sheet, at)
                .ok()
                .flatten()
                .unwrap_or_default(),
            format: self.app.format_at(sheet, at).ok().flatten(),
            shown,
        }
    }

    /// Write `write` over the selection — a whole row or column cut to what the sheet uses
    /// (`nav::target`), the rule every operation over a selection follows.
    fn write(&self, write: Write) {
        let sheet = self.sheet.get();
        let used = self.app.used_extent(sheet).unwrap_or((0, 0));
        let (start, end) = grind_sheet::nav::target(self.selection.get(), used);
        let done = match write {
            Write::Style(style) => self.app.set_style(sheet, start, end, style),
            Write::Format(format) => self.app.set_format(sheet, start, end, format),
            Write::Clear => self
                .app
                .set_style(sheet, start, end, None)
                .and_then(|_| self.app.set_format(sheet, start, end, None)),
        };
        match done {
            Ok(_) => self.say(None),
            Err(error) => self.say(Some((&error.to_string(), None))),
        }
    }

    /// A Format command over the selected cells.
    pub fn format(&self, command: Command) {
        let locale = grind_sheet::locale::from_environment();
        if let Some(write) = cells::write(command, &self.active(), locale) {
            self.write(write);
        }
    }

    /// Whether the command's menu item is ticked for the active cell — or, for View ▸ Friendly
    /// Formulas, whether formulas are being read in plain English.
    pub fn format_checked(&self, command: Command) -> bool {
        match command {
            Command::FriendlyFormulas => self.friendly.get(),
            Command::Filter => self.app.filter(self.sheet.get()).is_ok_and(|f| f.is_some()),
            Command::ShowFormulas => self.formulas.get(),
            Command::CellRoles => self.overlays.get().roles,
            Command::Names => self.overlays.get().names,
            _ => cells::checked(command, &self.active()),
        }
    }

    /// The font the font panel is shown for the active cell.
    fn panel_font(&self, manager: &NSFontManager) -> Retained<NSFont> {
        let style = self.active().style;
        let size =
            BASE_PT * grind_sheet::look::font_scale(style.font_size.as_deref()).unwrap_or(1.0);
        ns_font(
            manager,
            None,
            size,
            grind_sheet::look::bold_weight(style.font_weight.as_deref()),
            grind_sheet::look::italic_style(style.font_style.as_deref()),
        )
    }

    /// `changeFont:` — the font panel's answer, over the selected cells.
    pub fn change_font(&self, mtm: MainThreadMarker) {
        let manager = NSFontManager::sharedFontManager(mtm);
        let shown = self.panel_font(&manager);
        let answered = manager.convertFont(&shown);
        let active = self.active();
        if let Some(style) = cells::font_restyle(
            &active.style,
            &facts(&manager, &shown),
            &facts(&manager, &answered),
        ) {
            self.write(Write::Style(style));
        }
    }

    /// `changeColor:` — the colour panel's colour as the selected cells' text colour, which is
    /// what the panel means in every Mac application that has text.
    pub fn change_color(&self, color: &NSColor) {
        let style =
            grind_sheet::format::coloured(&self.active().style, false, Some(hex(rgb(color))));
        self.write(Write::Style(style));
    }

    /// Tell the font panel what the active cell is set in, so it opens on it.
    pub fn show_font(&self, mtm: MainThreadMarker) {
        let manager = NSFontManager::sharedFontManager(mtm);
        manager.setSelectedFont_isMultiple(&self.panel_font(&manager), false);
    }
}

impl TextPane {
    /// What the selection agrees about, or what the next character typed will carry.
    fn here(&self) -> CharStyle {
        let state = self.state.borrow();
        text_format::here(
            &self.app,
            state.selection(),
            state.caret,
            state.resume.as_ref(),
        )
    }

    /// The kind of the caret's block.
    /// The caret's block's kind, and the paragraph style it wears.
    fn block_kind(&self) -> (BlockKind, Option<String>) {
        let block = self.state.borrow().caret.block;
        self.app
            .get_viewport(block..block + 1)
            .get(block)
            .map_or((BlockKind::Paragraph, None), |view| {
                (view.kind.clone(), view.style.clone())
            })
    }

    /// Character changes over the selection, as one write — or held for the next character
    /// typed.
    fn apply(&self, changes: &[text_format::Change]) {
        self.act_on(|page, app, _| {
            let landed = text_format::apply_all(
                app,
                page.selection(),
                page.caret,
                page.resume.as_ref(),
                changes,
            )
            .map_err(|error| error.to_string())?;
            if let Landed::Pending(style) = landed {
                page.resume = Some(style);
            }
            Ok(())
        });
    }

    /// A Format command over the selection: a character change, or a paragraph kind for every
    /// block the selection touches.
    ///
    /// ponytail: a kind over several blocks is one `set_kind` each, so as many ⌘Z as blocks.
    /// `App` has no kind over a range; the Windows pane and the GNOME window pay the same, and
    /// the trigger is a third page that sets kinds over a selection.
    pub fn format(&self, command: Command) {
        if page::block(command, &BlockKind::Paragraph, None).is_some() {
            self.act_on(|state, app, _| {
                let (from, to) = state.selection().unwrap_or((state.caret, state.caret));
                let viewport = app.get_viewport(from.block..to.block + 1);
                for view in viewport.iter() {
                    let Some((kind, style)) =
                        page::block(command, &view.kind, view.style.as_deref())
                    else {
                        continue;
                    };
                    if kind != view.kind {
                        app.set_kind(view.index, kind)
                            .map_err(|error| error.to_string())?;
                    }
                    if style != view.style {
                        app.set_style(view.index..view.index + 1, style)
                            .map_err(|error| error.to_string())?;
                    }
                }
                Ok(())
            });
            return;
        }
        if let Some(change) = page::change(command, &self.here()) {
            self.apply(&[change]);
        }
    }

    /// Whether the command's menu item is ticked.
    pub fn format_checked(&self, command: Command) -> bool {
        match command {
            Command::Names => self.names.get(),
            _ => {
                let (kind, style) = self.block_kind();
                page::checked(command, &self.here(), &kind, style.as_deref())
            }
        }
    }

    /// The font the font panel is shown: the run's own family, or the block's face.
    fn panel_font(&self, manager: &NSFontManager) -> Retained<NSFont> {
        let here = self.here();
        let (kind, style) = self.block_kind();
        let role = grind_text::look::Role::of(&kind, style.as_deref());
        let font = face::font(role, &here.metrics());
        ns_font(
            manager,
            here.font_family.as_deref(),
            font.size,
            font.bold,
            font.italic,
        )
    }

    /// `changeFont:` — the font panel's answer, as the changes it made.
    pub fn change_font(&self, mtm: MainThreadMarker) {
        let manager = NSFontManager::sharedFontManager(mtm);
        let shown = self.panel_font(&manager);
        let answered = manager.convertFont(&shown);
        let changes = page::font_changes(&facts(&manager, &shown), &facts(&manager, &answered));
        if !changes.is_empty() {
            self.apply(&changes);
        }
    }

    /// `changeColor:` — the colour panel's colour as the text's.
    pub fn change_color(&self, color: &NSColor) {
        self.apply(&[text_format::Change::Color(Some(hex(rgb(color))))]);
    }

    /// Tell the font panel what the selection is set in, so it opens on it.
    pub fn show_font(&self, mtm: MainThreadMarker) {
        let manager = NSFontManager::sharedFontManager(mtm);
        manager.setSelectedFont_isMultiple(&self.panel_font(&manager), false);
    }
}

/// What the toolbar reads and writes on a pane — both panes answer it, so `toolbar.rs` knows
/// nothing about either.
pub trait Formats {
    fn format(&self, command: Command);
    fn checked(&self, command: Command) -> bool;
    /// The colour the selection's text is and the one behind it, as a document stores them —
    /// what the two wells show.
    fn colors(&self) -> (Option<String>, Option<String>);
    /// A colour from a well: the text's, or with `background` the fill's or highlight's.
    fn set_color(&self, color: &NSColor, background: bool);
    /// Call `listener` whenever what the toolbar shows may have changed.
    fn watch(&self, listener: Box<dyn Fn()>);
}

impl Formats for Pane {
    fn format(&self, command: Command) {
        Pane::format(self, command);
    }

    fn checked(&self, command: Command) -> bool {
        self.format_checked(command)
    }

    fn colors(&self) -> (Option<String>, Option<String>) {
        let style = self.active().style;
        (style.color, style.background)
    }

    fn set_color(&self, color: &NSColor, background: bool) {
        let style =
            grind_sheet::format::coloured(&self.active().style, background, Some(hex(rgb(color))));
        self.write(Write::Style(style));
    }

    fn watch(&self, listener: Box<dyn Fn()>) {
        self.listen(move |_| listener());
    }
}

impl Formats for TextPane {
    fn format(&self, command: Command) {
        TextPane::format(self, command);
    }

    fn checked(&self, command: Command) -> bool {
        self.format_checked(command)
    }

    fn colors(&self) -> (Option<String>, Option<String>) {
        let here = self.here();
        (here.color, here.background)
    }

    fn set_color(&self, color: &NSColor, background: bool) {
        let value = Some(hex(rgb(color)));
        self.apply(&[match background {
            true => text_format::Change::Highlight(value),
            false => text_format::Change::Color(value),
        }]);
    }

    fn watch(&self, listener: Box<dyn Fn()>) {
        self.listen(listener);
    }
}
