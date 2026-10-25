// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Formatting: direct character formatting, links, and named paragraph styles.

use crate::*;

impl App {
    /// The character formatting over a span — what a toolbar shows: whatever every character
    /// between the carets agrees about, so half-bold reads as not bold. An empty span reports the
    /// run at the caret, preferring the one to its left — [`App::insert_text`]'s rule, so the
    /// toolbar shows what the next keystroke produces.
    pub fn char_style(&self, from: Caret, to: Caret) -> Result<CharStyle> {
        let state = self.state.read().unwrap();
        if to < from {
            return Err(Error::Xml("that range runs backwards".to_owned()));
        }
        let mut common: Option<CharStyle> = None;
        for index in from.block..=to.block {
            let block = state
                .doc
                .block(index)
                .ok_or_else(|| Error::Xml(format!("no block {}", loc::format(index))))?;
            let start = if index == from.block { from.offset } else { 0 };
            let end = if index == to.block {
                to.offset
            } else {
                block.len()
            };
            for props in spanned(block, start, end) {
                common = Some(match common {
                    Some(so_far) => so_far.common(&props),
                    None => props,
                });
            }
        }
        Ok(common.unwrap_or_default())
    }

    /// Replace the character formatting of every character between two carets — bold, italic,
    /// font, size — as one undo step; returns how many blocks changed. It replaces rather than
    /// adds, as `grind_sheet::App::set_style` does: an empty [`CharStyle`] is plain again. Named
    /// character styles are untouched ([`crate::style`]).
    pub fn set_char_style(&self, from: Caret, to: Caret, style: &CharStyle) -> Result<usize> {
        self.mutate(|state| {
            if to < from {
                return Err(Error::Xml("that range runs backwards".to_owned()));
            }
            let mut batch = Vec::new();
            for index in from.block..=to.block {
                let mut block = block_at(state, index)?;
                let start = if index == from.block { from.offset } else { 0 };
                let end = if index == to.block {
                    to.offset
                } else {
                    block.len()
                };
                let Some(runs) = restyled(&block, start, end, style) else {
                    continue;
                };
                block.runs = runs;
                batch.push(Action::SetBlock {
                    index,
                    block: Box::new(block),
                });
            }
            let changed = batch.len();
            if changed > 0 {
                Self::commit(state, Action::Batch(batch))?;
            }
            Ok(changed)
        })
    }

    /// The hyperlink every character between two carets agrees about, or `None`. An empty span
    /// asks about the link the caret touches ([`App::link_at`]), so a caret at a link's end reads
    /// as on it.
    pub fn link(&self, from: Caret, to: Caret) -> Result<Option<String>> {
        if to < from {
            return Err(Error::Xml("that range runs backwards".to_owned()));
        }
        if from == to {
            return Ok(self.link_at(from)?.map(|link| link.href));
        }
        let state = self.state.read().unwrap();
        let mut common: Option<Option<String>> = None;
        for index in from.block..=to.block {
            let block = state
                .doc
                .block(index)
                .ok_or_else(|| Error::Xml(format!("no block {}", loc::format(index))))?;
            let start = if index == from.block { from.offset } else { 0 };
            let end = if index == to.block {
                to.offset
            } else {
                block.len()
            };
            let (_, rest) = model::split_runs(&block.runs, start);
            let (middle, _) = model::split_runs(&rest, end.saturating_sub(start));
            for run in &middle {
                if let Run::Text { href, .. } = run {
                    match &common {
                        Some(so_far) if so_far != href => return Ok(None),
                        Some(_) => {}
                        None => common = Some(href.clone()),
                    }
                }
            }
        }
        Ok(common.flatten())
    }

    /// The whole link at a caret — its extent and target — or `None`. The character to the left
    /// decides first, then the right, so a caret at either end finds it. A link never spans
    /// paragraphs (`text:a` is inline, rng:16453).
    pub fn link_at(&self, at: Caret) -> Result<Option<Link>> {
        let state = self.state.read().unwrap();
        let block = state
            .doc
            .block(at.block)
            .ok_or_else(|| Error::Xml(format!("no block {}", loc::format(at.block))))?;
        // Each text run's span and target, in caret offsets.
        let mut spans = Vec::new();
        let mut pos = 0usize;
        for run in &block.runs {
            let len = run.len();
            spans.push((pos, pos + len, run_href(run)));
            pos += len;
        }
        let offset = at.offset.min(pos);
        let touching = spans
            .iter()
            .position(|(start, end, href)| href.is_some() && *start < offset && offset <= *end)
            .or_else(|| {
                spans.iter().position(|(start, end, href)| {
                    href.is_some() && *start <= offset && offset < *end
                })
            });
        let Some(found) = touching else {
            return Ok(None);
        };
        let href = spans[found].2;
        // A link is every adjacent run with the same target: `[a **bold** word](url)` is three
        // runs and one link.
        let mut first = found;
        while first > 0 && spans[first - 1].2 == href {
            first -= 1;
        }
        let mut last = found;
        while last + 1 < spans.len() && spans[last + 1].2 == href {
            last += 1;
        }
        Ok(Some(Link {
            from: Caret {
                block: at.block,
                offset: spans[first].0,
            },
            to: Caret {
                block: at.block,
                offset: spans[last].1,
            },
            href: href.unwrap_or_default().to_owned(),
        }))
    }

    /// Link every character between two carets to `href` — a URL, or `#name` for a bookmark — or
    /// with `None` unlink them, as one undo step; formatting is untouched. Returns how many blocks
    /// changed. An empty `href` is refused: `xlink:href` is required on `text:a` (rng:16453).
    pub fn set_link(&self, from: Caret, to: Caret, href: Option<&str>) -> Result<usize> {
        if href.is_some_and(|href| href.trim().is_empty()) {
            return Err(Error::Xml("a link needs somewhere to point".to_owned()));
        }
        self.mutate(|state| {
            if to < from {
                return Err(Error::Xml("that range runs backwards".to_owned()));
            }
            let mut batch = Vec::new();
            for index in from.block..=to.block {
                let mut block = block_at(state, index)?;
                let start = if index == from.block { from.offset } else { 0 };
                let end = if index == to.block {
                    to.offset
                } else {
                    block.len()
                };
                let Some(runs) = relinked(&block, start, end, href) else {
                    continue;
                };
                block.runs = runs;
                batch.push(Action::SetBlock {
                    index,
                    block: Box::new(block),
                });
            }
            let changed = batch.len();
            if changed > 0 {
                Self::commit(state, Action::Batch(batch))?;
            }
            Ok(changed)
        })
    }

    /// Set — or with `None`, clear — the named paragraph style of a run of blocks.
    pub fn set_style(&self, blocks: Range<usize>, style: Option<String>) -> Result<usize> {
        self.mutate(|state| {
            if blocks.end > state.doc.blocks.len() {
                return Err(Error::Xml("that range runs past the end".to_owned()));
            }
            let mut batch = Vec::new();
            for index in blocks {
                let mut block = block_at(state, index)?;
                if block.style == style {
                    continue;
                }
                block.style = style.clone();
                batch.push(Action::SetBlock {
                    index,
                    block: Box::new(block),
                });
            }
            let changed = batch.len();
            if changed > 0 {
                Self::commit(state, Action::Batch(batch))?;
            }
            Ok(changed)
        })
    }
}
