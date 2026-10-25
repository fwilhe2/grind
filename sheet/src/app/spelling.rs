// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later
//! Spelling in a spreadsheet (`doc/spelling.md`, "In a spreadsheet"): which words in the cells are
//! misspelt, what they might have been, and correcting one.

use crate::*;

impl App {
    /// Check spelling against `lexicon` from now on, or with `None` stop. Observers are not told,
    /// since the document has not changed; the caller redraws.
    pub fn set_lexicon(&self, lexicon: Option<Arc<dyn spell::Lexicon>>) {
        *self.lexicon.write().unwrap() = lexicon;
    }

    /// The language spelling is being checked in — the attached lexicon's BCP 47 tag — or `None`
    /// when nothing is checked.
    pub fn spelling_language(&self) -> Option<String> {
        let lexicon = self.lexicon.read().unwrap();
        lexicon.as_ref().map(|l| l.language().to_owned())
    }

    /// Every misspelt word in one sheet, or every sheet with `None`, in reading order; empty
    /// without a dictionary. `within` narrows it to a rectangle on whichever sheets are checked.
    /// Only typed text is checked ([`Sheet::text_cells`]).
    pub fn misspellings(
        &self,
        sheet: Option<usize>,
        within: Option<(Pos, Pos)>,
    ) -> Result<Vec<spell::Misspelling>> {
        let Some(lexicon) = self.lexicon.read().unwrap().clone() else {
            return Ok(Vec::new());
        };
        let state = self.state.read().unwrap();
        let sheets = match sheet {
            Some(sheet) => {
                state.doc.sheet(sheet).ok_or(Error::NoSuchSheet(sheet))?;
                sheet..sheet + 1
            }
            None => 0..state.doc.sheets.len(),
        };
        Ok(sheets
            .flat_map(|i| spell::check_sheet(i, &state.doc.sheets[i], within, lexicon.as_ref()))
            .collect())
    }

    /// What `word` might have been, best first; empty without a dictionary. Tens of milliseconds,
    /// so one word at a time.
    pub fn suggest(&self, word: &str) -> Vec<String> {
        let lexicon = self.lexicon.read().unwrap().clone();
        lexicon.map_or_else(Vec::new, |l| l.suggest(&spell::normalise(word)))
    }

    /// Replace the `word` at character `offset` of a text cell with `with`, as one undo step,
    /// recalculating what reads it. The cell stays text and keeps its style and format. Refused if
    /// the cell no longer holds `word` there.
    pub fn correct(
        &self,
        sheet: usize,
        pos: Pos,
        offset: usize,
        word: &str,
        with: &str,
    ) -> Result<Option<Recalc>> {
        self.mutate(|state| {
            let s = state.doc.sheet(sheet).ok_or(Error::NoSuchSheet(sheet))?;
            let at = a1::format(Some(&s.name), pos);
            let text = match (s.get(pos), s.formula(pos)) {
                (CellValue::Text(text), None) => text,
                _ => return Err(Error::Stale(format!("{at} holds no typed text"))),
            };
            let chars: Vec<char> = text.chars().collect();
            let len = word.chars().count();
            let found: String = chars.iter().skip(offset).take(len).collect();
            if found != word || offset + len > chars.len() {
                return Err(Error::Stale(format!(
                    "{at}+{offset} reads {found:?}, not {word:?}"
                )));
            }
            let corrected: String = chars[..offset]
                .iter()
                .copied()
                .chain(with.chars())
                .chain(chars[offset + len..].iter().copied())
                .collect();
            let edit = Action::SetFormula {
                sheet,
                pos,
                formula: None,
                value: CellValue::Text(corrected),
            };
            commit(state, sheet, vec![edit], RecalcMode::Document)
        })
    }
}
