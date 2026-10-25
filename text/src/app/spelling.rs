// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Spelling: the lexicon a shell attaches, and checking and correcting against it
//! (`doc/spelling.md`).

use crate::*;

impl App {
    /// Check spelling against `lexicon` from now on, or with `None` stop. Observers are not told,
    /// since the document has not changed and a shell would mark a freshly opened file modified;
    /// the caller redraws.
    pub fn set_lexicon(&self, lexicon: Option<Arc<dyn spell::Lexicon>>) {
        *self.lexicon.write().unwrap() = lexicon;
    }

    /// The language spelling is being checked in — the attached lexicon's BCP 47 tag — or `None`
    /// when nothing is checked.
    pub fn spelling_language(&self) -> Option<String> {
        let lexicon = self.lexicon.read().unwrap();
        lexicon.as_ref().map(|l| l.language().to_owned())
    }

    /// The language the document states: `fo:language`/`fo:country` of its `Standard` paragraph
    /// style, inheriting the default paragraph style's, where LibreOffice keeps it. `de-DE`, `en`,
    /// or `None`; `zxx`, ODF's "no language", comes back as itself.
    pub fn language(&self) -> Option<String> {
        let state = self.state.read().unwrap();
        let props = paragraph::resolve(
            &state.doc.paragraph_styles,
            &state.doc.default_paragraph,
            Some("Standard"),
        );
        let language = props.language.filter(|l| !l.is_empty() && l != "none")?;
        Some(
            match props.country.filter(|c| !c.is_empty() && c != "none") {
                Some(country) => format!("{language}-{country}"),
                None => language,
            },
        )
    }

    /// Every misspelt word in `blocks`, in document order; empty without a lexicon.
    pub fn misspellings(&self, blocks: Range<usize>) -> Vec<spell::Misspelling> {
        let Some(lexicon) = self.lexicon.read().unwrap().clone() else {
            return Vec::new();
        };
        let state = self.state.read().unwrap();
        let end = blocks.end.min(state.doc.blocks.len());
        let start = blocks.start.min(end);
        state.doc.blocks[start..end]
            .iter()
            .enumerate()
            .flat_map(|(i, block)| spell::check_block(block, start + i, lexicon.as_ref()))
            .collect()
    }

    /// What `word` might have been, best first; empty without a lexicon. Tens of milliseconds, so
    /// one word at a time.
    pub fn suggest(&self, word: &str) -> Vec<String> {
        let lexicon = self.lexicon.read().unwrap().clone();
        lexicon.map_or_else(Vec::new, |l| l.suggest(&spell::normalise(word)))
    }

    /// Replace the `word` at `at` with `with`, as one undo step, keeping the formatting of the
    /// word's first character. Refused if `word` is no longer there.
    pub fn correct(&self, at: Caret, word: &str, with: &str) -> Result<()> {
        self.mutate(|state| {
            let mut block = block_at(state, at.block)?;
            let len = word.chars().count();
            let (mut runs, rest) = model::split_runs(&block.runs, at.offset.min(block.len()));
            let (cut, tail) = model::split_runs(&rest, len);
            let found: String = cut.iter().map(Run::text).collect();
            if found != word {
                return Err(Error::Xml(format!(
                    "{}+{} reads {found:?}, not {word:?}",
                    loc::format(at.block),
                    at.offset
                )));
            }
            let (style, props, href) = cut
                .iter()
                .find_map(|run| match run {
                    Run::Text {
                        style, props, href, ..
                    } => Some((style.clone(), props.clone(), href.clone())),
                    _ => None,
                })
                .unwrap_or_default();
            if !with.is_empty() {
                runs.push(Run::Text {
                    text: with.to_owned(),
                    style,
                    props,
                    href,
                });
            }
            runs.extend(
                cut.into_iter()
                    .filter(|r| matches!(r, Run::Bookmark { .. })),
            );
            runs.extend(tail);
            model::coalesce(&mut runs);
            block.runs = runs;
            Self::commit(
                state,
                Action::SetBlock {
                    index: at.block,
                    block: Box::new(block),
                },
            )
        })
    }
}
