// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! A text document, typeset onto pages — the word processor's half of this crate.
//!
//! [`typeset`] is the whole pipeline before a backend: the page geometry (the document's own,
//! or the one asked for, or A4), [`grind_text::page::paginate`] with the paper faces, and then
//! each placed line cut into pieces of one formatting ([`grind_text::paint::pieces`], the cut
//! every shell's painter uses) and shaped by the same [`Typesetter`] that measured it.

use std::collections::HashMap;
use std::sync::Arc;

use grind_core::layout::{Fragment, wrap};
use grind_core::page::{PageGeometry, pt};
use grind_core::style::{TextStyle, length_mm};
use grind_text::flow::{self, CellBox};
use grind_text::page::{Paginated, Piece, Rules, paginate_slots};
use grind_text::{App, BlockKind, BlockView, Caret, paint, picture_of};

use crate::faces::{Column, RoleFace, SPACING, role_faces};
use crate::metrics::Typesetter;
use crate::ops::{Document, Element, Heading, Mark, Op, Page, Rgb};
use grind_core::layout::Align;

use crate::place::LineBox;

mod marginal;
mod notes;

use marginal::{Boxes, Marginal};
use notes::{endnote_pages, note_band};

/// The gap between a picture and its caption, in points.
const CAPTION_GAP: f64 = 4.0;
/// The ink of a link that states no colour: Writer's own for an unstyled hyperlink.
const LINK: Rgb = Rgb(0, 0, 0x80);
/// A table's rules, in points: a hairline that still prints.
const RULE: f32 = 0.5;

/// What the caller may decide about the pages.
#[derive(Clone, Debug, Default)]
pub struct Options {
    /// The paper to print on, overriding the document's own. `None` prints on the page the
    /// document states, and on A4 when it states none.
    pub paper: Option<PageGeometry>,
}

/// Typeset `app`'s document with `setter`'s fonts.
pub fn typeset(app: &App, setter: &Typesetter, options: &Options) -> Document {
    typeset_after(app, setter, options, None)
}

/// [`typeset`], drawing only the pages that differ from `previous` — the same document set
/// before an edit, with the same `setter`. A page is the same when everything its drawing reads
/// is: its place in the flow, its blocks and their styles, labels, boxes and spacing, its number
/// and running heads, and the pages' masters. The rest are `previous`'s, copied
/// (`doc/page-model.md` PM3). An edit that left every block it changed the shape it was is
/// not even cut again: only the pages holding it are drawn.
pub fn typeset_after(
    app: &App,
    setter: &Typesetter,
    options: &Options,
    previous: Option<&Document>,
) -> Document {
    setter.hint(app.font_generics());
    // Endnote pages follow the cut ones, which setting again in part does not know of.
    let endnotes: Vec<grind_text::marginal::Note> = app
        .notes()
        .into_iter()
        .filter_map(|(_, note)| note.endnote.then_some(note))
        .collect();
    if let Some(previous) = previous.filter(|_| endnotes.is_empty())
        && let Some(set) = touch_up(app, setter, options, previous)
            .or_else(|| recut(app, setter, options, previous))
    {
        return set;
    }
    let revision = app.revision();
    let viewport = app.get_viewport(0..app.block_count());
    let (kinds, starts) = page_kinds(app, &viewport);
    let paper = Paper::new(app, setter, options, kinds);
    let maps = Maps::over(app, setter, &viewport, &paper.widths);
    let columns = maps.columns();
    let column = &columns[0];
    let measures: Vec<(&dyn grind_text::Faces, f64)> = columns
        .iter()
        .map(|column| (column as &dyn grind_text::Faces, column.width))
        .collect();
    let picture =
        |view: &BlockView, measure: f64| maps.figure(view, measure).map(|figure| figure.height);
    let Paginated { pages, layouts } = paginate_slots(
        (app, &viewport),
        &measures,
        &paper.slots,
        (0, &starts),
        None,
        &SPACING,
        breaking(app),
        &picture,
    );
    let numbering = numbering(&pages);
    let chapters = chapters(Vec::new(), &pages, &viewport);

    let kept: HashMap<u64, &Arc<Page>> = previous
        .map_or_default(|previous| previous.keys.iter().copied().zip(&previous.pages).collect());
    let mut outline = Vec::new();
    let mut keys = Vec::with_capacity(pages.len());
    let mut relative = Vec::with_capacity(pages.len());
    let drawn = pages
        .iter()
        .enumerate()
        .map(|(number, page)| {
            let heads = (numbering[number], chapters[number].as_slice());
            relative.push(maps.key(column, paper.seed, page, heads));
            let key = page_key(page, relative[number]);
            keys.push(key);
            outline.extend(maps.headings(&paper.sheets[page.slot], page, number));
            match kept.get(&key) {
                // Drawn before as it is now.
                Some(same) => Arc::clone(same),
                None => Arc::new(maps.draw(column, &paper.sheets[page.slot], page, heads)),
            }
        })
        .collect();
    let mut drawn: Vec<Arc<Page>> = drawn;
    if let Some(last) = pages.last()
        && !endnotes.is_empty()
    {
        let sheet = &paper.sheets[last.slot];
        // Set straight after the body's last line, as Word does, when the document says so.
        let after = app
            .note_look()
            .continuous
            .then_some(())
            .and_then(|()| drawn.last_mut())
            .map(|page| {
                let foot = page
                    .lines
                    .iter()
                    .map(|line| f64::from(line.top + line.height))
                    .fold(sheet.top, f64::max);
                (Arc::make_mut(page), foot, sheet.top + sheet.height)
            });
        for (at, page) in endnote_pages(app, setter, &endnotes, sheet.geometry, after)
            .into_iter()
            .enumerate()
        {
            keys.push(page_key(last, at as u64 ^ 0x656e_646e_6f74_6573));
            drawn.push(Arc::new(page));
        }
    }
    let shapes = viewport
        .iter()
        .map(|view| maps.shape(view, layouts.get(view.index)?.as_ref()?))
        .collect();
    Document {
        pages: drawn,
        outline,
        structure: structure(&viewport),
        keys,
        cut: Some(Arc::new(Cut {
            revision,
            seed: paper.seed,
            kinds: paper.kinds,
            starts: Arc::new(starts),
            pages: pages.into(),
            numbering: numbering.into(),
            chapters: chapters.into(),
            shapes,
            relative: relative.into(),
        })),
    }
}

/// How a setting cut its pages, kept with them for the next setting to start from.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Cut {
    /// The document's revision when it was set ([`App::changed_since`]).
    revision: u64,
    /// [`Paper::seed`].
    seed: u64,
    kinds: Vec<Kind>,
    starts: Arc<grind_text::page::Starts>,
    pages: Arc<[grind_text::page::Page]>,
    numbering: Arc<[(usize, usize)]>,
    chapters: Arc<[Vec<String>]>,
    /// Each block's [`Maps::shape`] as it was cut.
    shapes: Arc<[Option<Shape>]>,
    /// Each page's [`Maps::key`], before [`page_key`] says where the page is.
    relative: Arc<[u64]>,
}

/// What of a block the cut reads, hashed: its kind, style, label and breaks, and apart from
/// them its lines' tops and heights; and whether other blocks count from it — a list item,
/// whose list numbers on from it, or a paragraph opening a run of pages.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Shape {
    look: u64,
    lines: u64,
    counted: bool,
}

/// The document's own widows and orphans, and none where it states none — what Writer does
/// (`doc/odt-format.md` §5c, fact 3).
fn breaking(app: &App) -> Rules {
    let stated = app.paragraph_defaults();
    Rules {
        orphans: stated.orphans.unwrap_or(0) as usize,
        widows: stated.widows.unwrap_or(0) as usize,
    }
}

/// `previous` set again after an edit that left every block it changed the shape it was: then
/// the pages are cut as they were, and only those holding a changed block are drawn again, from
/// what is read about their blocks and the one either side of them. `None` when the edit added
/// or removed a block, changed one's shape, or anything else; the whole document is set then.
fn touch_up(
    app: &App,
    setter: &Typesetter,
    options: &Options,
    previous: &Document,
) -> Option<Document> {
    let cut = previous.cut.as_deref()?;
    let revision = app.revision();
    let changed = app.changed_since(cut.revision)?;
    if changed.blocks.len() != changed.before {
        return None;
    }
    let paper = Paper::new(app, setter, options, cut.kinds.clone());
    if paper.seed != cut.seed {
        return None;
    }
    let touched: Vec<usize> = (0..cut.pages.len())
        .filter(|&number| blocks_of(&cut.pages[number]).any(|i| changed.blocks.contains(&i)))
        .collect();
    let on_them = || {
        touched
            .iter()
            .flat_map(|&number| blocks_of(&cut.pages[number]))
    };
    let (Some(first), Some(last)) = (on_them().min(), on_them().max()) else {
        let mut set = previous.clone();
        set.cut = Some(Arc::new(Cut {
            revision,
            ..cut.clone()
        }));
        return changed.none().then_some(set);
    };
    // What a block's neighbours decide of it (contextual spacing, joined borders, the larger
    // space) reaches one block; a table's columns are worked out over the whole table.
    let mut window = first.saturating_sub(1)..(last + 2).min(app.block_count());
    if let Some(table) = app.table(window.start) {
        window.start = window.start.min(table.blocks.start);
    }
    if let Some(table) = window.end.checked_sub(1).and_then(|end| app.table(end)) {
        window.end = window.end.max(table.blocks.end);
    }
    let viewport = app.get_viewport(window);
    let maps = Maps::over(app, setter, &viewport, &paper.widths);
    let columns = maps.columns();
    let column = &columns[0];
    for index in changed.blocks.clone() {
        let view = viewport.get(index)?;
        let piece = touched
            .iter()
            .flat_map(|&number| &cut.pages[number].pieces)
            .find(|piece| piece.index == index)?;
        let layout = maps.layout(column, view, piece.width)?;
        let shape = maps.shape(view, &layout);
        if shape.is_none() || shape != *cut.shapes.get(index)? {
            return None;
        }
    }
    let mut set = previous.clone();
    let mut relative = cut.relative.to_vec();
    for number in touched {
        let page = &cut.pages[number];
        let heads = (cut.numbering[number], cut.chapters[number].as_slice());
        relative[number] = maps.key(column, cut.seed, page, heads);
        let key = page_key(page, relative[number]);
        if key != set.keys[number] {
            set.pages[number] = Arc::new(maps.draw(column, &paper.sheets[page.slot], page, heads));
            set.keys[number] = key;
        }
    }
    set.cut = Some(Arc::new(Cut {
        revision,
        relative: relative.into(),
        ..cut.clone()
    }));
    Some(set)
}

/// `previous` cut again after an edit that changed blocks' lines, or added or took out plain
/// paragraphs: from the last page before the change that opens on a whole block, over a window
/// of blocks, to the first page that opens where one did before — on the same block, kind of
/// page and number. The pages from there are the ones before, moved by the blocks the edit
/// added or took out. `None` when the edit could have changed what other blocks show (a list
/// item's number, a heading, a run of pages), no such page turns up near the change, or the
/// pages are numbered differently; the whole document is set then.
fn recut(
    app: &App,
    setter: &Typesetter,
    options: &Options,
    previous: &Document,
) -> Option<Document> {
    let cut = previous.cut.as_deref()?;
    let revision = app.revision();
    let changed = app.changed_since(cut.revision)?;
    if changed.none() {
        return None;
    }
    let by = isize::try_from(changed.blocks.len()).ok()? - isize::try_from(changed.before).ok()?;
    let was = changed.blocks.start..changed.blocks.start + changed.before;
    let paper = Paper::new(app, setter, options, cut.kinds.clone());
    if paper.seed != cut.seed {
        return None;
    }
    let count = app.block_count();
    // The page the change is on, and the last at or before it opening on a whole block two
    // before the change: where a page opens was decided by its first block and, when that one
    // keeps with the next, the block after it.
    let on = cut
        .pages
        .iter()
        .position(|page| blocks_of(page).any(|index| index >= changed.blocks.start))?;
    let from = (1..=on)
        .rev()
        .find(|&number| opening(&cut.pages[number]).is_some_and(|i| i + 2 <= changed.blocks.start))
        .unwrap_or(0);
    let first = if from == 0 {
        0
    } else {
        opening(&cut.pages[from])?
    };
    let after = (from > 0).then(|| (&cut.pages[from - 1], cut.pages[from].pieces[0].top));
    // A run of pages is set at the measure of the page that opened it.
    let run = (cut.starts.iter())
        .filter(|&(&index, _)| index <= first)
        .max_by_key(|&(&index, _)| index)
        .map_or(0, |(_, &(slot, _))| slot);
    let starts: grind_text::page::Starts = (cut.starts.iter())
        .map(|(&index, &start)| {
            let moved = if index >= was.end {
                index.saturating_add_signed(by)
            } else {
                index
            };
            (moved, start)
        })
        .collect();
    let mut reach = 64;
    while reach <= 4096 {
        let mut end = (changed.blocks.end + reach).min(count);
        if let Some(table) = app.table(end.saturating_sub(1)) {
            end = end.max(table.blocks.end);
        }
        reach *= 4;
        let flowing = app.get_viewport(first..end);
        if flowing.get(first).is_some_and(|view| view.cell.is_some()) {
            return None;
        }
        let read = app.get_viewport(first.saturating_sub(1)..(end + 1).min(count));
        let maps = Maps::over(app, setter, &read, &paper.widths);
        let columns = maps.columns();
        let column = &columns[0];
        let measures: Vec<(&dyn grind_text::Faces, f64)> = columns
            .iter()
            .map(|column| (column as &dyn grind_text::Faces, column.width))
            .collect();
        let picture =
            |view: &BlockView, measure: f64| maps.figure(view, measure).map(|figure| figure.height);
        let Paginated { pages, layouts } = paginate_slots(
            (app, &flowing),
            &measures,
            &paper.slots,
            (run, &starts),
            after,
            &SPACING,
            breaking(app),
            &picture,
        );
        let shapes = changed
            .blocks
            .clone()
            .map(|index| maps.shape(flowing.get(index)?, layouts.get(index)?.as_ref()?))
            .collect::<Option<Vec<Shape>>>()?;
        let olds = cut.shapes.get(was.clone())?;
        // Each block as it was, only its lines changed; or plain paragraphs, none of which
        // another block counts from, for plain paragraphs.
        let same = by == 0
            && (olds.iter().zip(&shapes)).all(|(old, new)| old.is_some_and(|o| o.look == new.look));
        let plain = changed.before > 0
            && !changed.blocks.is_empty()
            && olds.iter().all(|old| old.is_some_and(|old| !old.counted))
            && shapes.iter().all(|new| !new.counted);
        if !same && !plain {
            return None;
        }
        // The first page past the change that opens as one did before, far enough from the
        // window's end that nothing after it was missing from its cut, and from the change that
        // its first block's neighbour is one the edit left alone.
        let met = pages.iter().enumerate().skip(1).find_map(|(at, page)| {
            let index = opening(page)?;
            if index <= changed.blocks.end
                || (end < count && index + 2 > end)
                || read.get(index).is_none_or(|view| view.cell.is_some())
            {
                return None;
            }
            let then = index.checked_add_signed(-by)?;
            let old = (from..cut.pages.len()).find(|&n| opening(&cut.pages[n]) == Some(then))?;
            let was = &cut.pages[old];
            (was.slot == page.slot
                && was.number == page.number
                && was.restart == page.restart
                && drawn(was.pieces[0].top) == drawn(page.pieces[0].top))
            .then_some((at, old))
        });
        let (new, old) = match met {
            Some(met) => met,
            None if end == count => (pages.len(), cut.pages.len()),
            None => continue,
        };
        let fresh = &pages[..new];
        let moved: Vec<grind_text::page::Page> = (cut.pages[old..].iter())
            .map(|page| moved_by(page, by))
            .collect();
        let cut_pages: Vec<grind_text::page::Page> = (cut.pages[..from].iter())
            .chain(fresh)
            .chain(&moved)
            .cloned()
            .collect();
        if numbering(&cut_pages) != *cut.numbering {
            return None;
        }
        let before = from.checked_sub(1).map(|n| cut.chapters[n].clone());
        let heads = chapters(before.unwrap_or_default(), fresh, &read);
        let mut set = previous.clone();
        let mut relative = cut.relative.to_vec();
        let mut outline = Vec::new();
        for (at, page) in fresh.iter().enumerate() {
            let number = from + at;
            let heads = (cut.numbering[number], heads[at].as_slice());
            relative[number] = maps.key(column, cut.seed, page, heads);
            let key = page_key(page, relative[number]);
            outline.extend(maps.headings(&paper.sheets[page.slot], page, number));
            if key != set.keys[number] {
                set.pages[number] =
                    Arc::new(maps.draw(column, &paper.sheets[page.slot], page, heads));
                set.keys[number] = key;
            }
        }
        if by != 0 {
            for number in old..cut_pages.len() {
                set.pages[number] = Arc::new(set.pages[number].shifted(by));
                set.keys[number] = page_key(&cut_pages[number], relative[number]);
            }
        }
        let (early, late): (Vec<Heading>, Vec<Heading>) = (previous.outline.iter())
            .filter(|&heading| heading.page < from || heading.page >= old)
            .cloned()
            .partition(|heading| heading.page < from);
        set.outline = early.into_iter().chain(outline).chain(late).collect();
        let span = structure(&read)
            .into_iter()
            .filter(|element| changed.blocks.contains(&element.block()));
        set.structure = (previous.structure[..was.start].iter().cloned())
            .chain(span)
            .chain(previous.structure[was.end..].iter().map(|e| e.shifted(by)))
            .collect();
        let chapters = (cut.chapters[..from].iter().cloned())
            .chain(heads)
            .chain(cut.chapters[old..].iter().cloned())
            .collect();
        let shapes = (cut.shapes[..was.start].iter().copied())
            .chain(shapes.into_iter().map(Some))
            .chain(cut.shapes[was.end..].iter().copied())
            .collect();
        set.cut = Some(Arc::new(Cut {
            revision,
            starts: Arc::new(starts),
            pages: cut_pages.into(),
            chapters,
            shapes,
            relative: relative.into(),
            ..cut.clone()
        }));
        return Some(set);
    }
    None
}

/// A page as cut, `by` blocks further on.
fn moved_by(page: &grind_text::page::Page, by: isize) -> grind_text::page::Page {
    let mut page = page.clone();
    for piece in &mut page.pieces {
        piece.index = piece.index.saturating_add_signed(by);
    }
    for cell in &mut page.cells {
        cell.first = cell.first.saturating_add_signed(by);
    }
    page
}

/// A place on the page at the precision it is drawn at. Where the flow is cut, it is a
/// difference of two places hundreds of pages down, whose last bits depend on where the cut
/// began; drawn, they are the same.
fn drawn(value: f64) -> u32 {
    (value as f32).to_bits()
}

/// The block a page opens with, when it opens with the first line of one that is not a
/// heading row repeated.
fn opening(page: &grind_text::page::Page) -> Option<usize> {
    let piece = page.pieces.first()?;
    (piece.lines.start == 0 && !piece.repeat).then_some(piece.index)
}

/// A page's key: its [`Maps::key`] and the block it starts from.
fn page_key(page: &grind_text::page::Page, relative: u64) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = key_hasher();
    (first_block(page), relative).hash(&mut hasher);
    hasher.finish()
}

/// The first block a page holds; 0 for an empty one.
fn first_block(page: &grind_text::page::Page) -> usize {
    blocks_of(page).min().unwrap_or(0)
}

/// The blocks a page holds, in a piece or a cell.
fn blocks_of(page: &grind_text::page::Page) -> impl Iterator<Item = usize> + '_ {
    let pieces = page.pieces.iter().map(|piece| piece.index);
    pieces.chain(page.cells.iter().map(|cell| cell.first))
}

/// The kinds of page the document is set on, laid out, and what a page's drawing reads of them.
struct Paper<'a> {
    kinds: Vec<Kind>,
    sheets: Vec<Sheet<'a>>,
    /// The widths text is set at: the document's own page first, then each master's that
    /// differs.
    widths: Vec<f64>,
    slots: Vec<grind_text::page::Slot>,
    /// What every page's drawing reads that is not the page's own: the paper, the page set, the
    /// widths text is set at — and the document's styles, masters and running heads, which only
    /// reading a document sets, so its generation names them.
    seed: u64,
}

impl<'a> Paper<'a> {
    fn new(app: &'a App, setter: &'a Typesetter, options: &Options, kinds: Vec<Kind>) -> Self {
        let default_geometry = options.paper.or(app.page()).unwrap_or_default();
        let sheets: Vec<Sheet<'_>> = kinds
            .iter()
            .map(|kind| Sheet::lay_out(app, setter, kind, default_geometry, options))
            .collect();
        let mut widths = vec![pt(default_geometry.text_width())];
        for sheet in &sheets {
            let own = pt(sheet.geometry.text_width());
            if !widths.iter().any(|w| (w - own).abs() < 1e-6) {
                widths.push(own);
            }
        }
        let slots = sheets
            .iter()
            .zip(&kinds)
            .map(|(sheet, kind)| grind_text::page::Slot {
                height: sheet.height,
                next: kind.next,
                measure: widths
                    .iter()
                    .position(|w| (w - pt(sheet.geometry.text_width())).abs() < 1e-6)
                    .unwrap_or(0),
                master: kind.master,
                plain: kind.plain,
                left: kind.left_twin,
                hand: kind.hand,
            })
            .collect();
        let seed = {
            use std::hash::{Hash, Hasher};
            let mut hasher = key_hasher();
            app.styles_generation().hash(&mut hasher);
            format!("{:?}|{:?}", options.paper, app.page()).hash(&mut hasher);
            for kind in &kinds {
                (&kind.name, kind.first, kind.left).hash(&mut hasher);
            }
            for width in &widths {
                width.to_bits().hash(&mut hasher);
            }
            hasher.finish()
        };
        Paper {
            kinds,
            sheets,
            widths,
            slots,
            seed,
        }
    }
}

/// What setting the blocks of a viewport reads about each: its face, room, label, box, tabs and
/// the rest, by block index, worked out over the viewport.
struct Maps<'a> {
    app: &'a App,
    setter: &'a Typesetter,
    viewport: &'a grind_text::Viewport,
    faces: Vec<RoleFace<'a>>,
    blocks: HashMap<usize, RoleFace<'a>>,
    spacing: HashMap<usize, crate::faces::Room>,
    breaks: HashMap<usize, grind_text::page::Breaks>,
    insets: HashMap<usize, f64>,
    indents: HashMap<usize, f32>,
    tabs: HashMap<usize, grind_core::layout::Tabs>,
    aligns: HashMap<usize, crate::faces::Aligned>,
    hyphenations: HashMap<usize, grind_core::layout::Hyphenate>,
    objects: HashMap<usize, Vec<(usize, grind_core::layout::Object)>>,
    /// Each block's footnotes, laid out, by the character each is cited at, and their heights
    /// in points.
    footnotes: HashMap<usize, Vec<(usize, Marginal<'a>)>>,
    note_heights: HashMap<usize, Vec<(usize, f64)>>,
    note_look: grind_text::marginal::NoteLook,
    looks: HashMap<String, grind_text::table_look::TableLook>,
    /// Each table's look, hashed once: a cell's drawing reads its table's.
    tables: HashMap<String, u64>,
    labels: HashMap<usize, Label>,
    frames: HashMap<usize, Frame>,
    /// One per width text is set at ([`Paper::widths`]).
    widths: Vec<f64>,
    across: Vec<HashMap<usize, flow::Across>>,
}

impl<'a> Maps<'a> {
    fn over(
        app: &'a App,
        setter: &'a Typesetter,
        viewport: &'a grind_text::Viewport,
        widths: &[f64],
    ) -> Self {
        let faces = role_faces(setter);
        let looks: HashMap<String, grind_text::table_look::TableLook> = viewport
            .iter()
            .filter_map(|view| view.cell.as_ref().map(|cell| cell.table.clone()))
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .filter_map(|table| Some((table.clone(), app.table_look(&table)?)))
            .collect();
        let blocks = block_faces(app, setter, viewport);
        let mut spacing = block_spacing(app, viewport);
        if !app.paragraph_spaces_add() {
            keep_the_larger_space(viewport, &mut spacing);
        }
        let breaks = block_breaks(app, viewport);
        let mut indents: HashMap<usize, f32> = viewport
            .iter()
            .filter_map(|view| {
                let resolved = app.paragraph_shared(view.index)?;
                let props = &resolved.props;
                let first = props.text_indent.as_deref().and_then(length_mm).map(pt)?;
                (first != 0.0).then_some((view.index, first as f32))
            })
            .collect();
        // Every paragraph's tab stops — its own and the default interval — in points.
        let tabs: HashMap<usize, grind_core::layout::Tabs> = viewport
            .iter()
            .filter(|view| view.text.contains('\t'))
            .filter_map(|view| {
                let tabs = app.paragraph_shared(view.index)?.props.tabs();
                (!tabs.is_empty()).then_some((view.index, tabs))
            })
            .collect();
        let aligns: HashMap<usize, crate::faces::Aligned> = viewport
            .iter()
            .filter_map(|view| {
                let resolved = app.paragraph_shared(view.index)?;
                let props = &resolved.props;
                let align = Align::parse(props.text_align.as_deref());
                let last = Align::parse(props.text_align_last.as_deref());
                let single = props.justify_single_word.as_deref() == Some("true");
                (align != Align::Start).then_some((view.index, (align, last, single)))
            })
            .collect();
        // Where the paragraph style asks for hyphenation, in its language (fact 55).
        let hyphenations: HashMap<usize, grind_core::layout::Hyphenate> = viewport
            .iter()
            .filter_map(|view| {
                let resolved = app.paragraph_shared(view.index)?;
                let props = &resolved.props;
                if props.hyphenate.as_deref() != Some("true") {
                    return None;
                }
                let count = |value: &Option<String>| {
                    value
                        .as_deref()
                        .and_then(|v| v.trim().parse().ok())
                        .unwrap_or(0)
                };
                Some((
                    view.index,
                    grind_core::layout::Hyphenate {
                        language: props.language.clone()?,
                        head: count(&props.hyphenation_remain),
                        tail: count(&props.hyphenation_push),
                    },
                ))
            })
            .collect();
        let objects = viewport
            .iter()
            .filter(|view| picture_of(view).is_none())
            .filter_map(|view| {
                let face = crate::faces::face_for(
                    &faces,
                    &blocks,
                    view.index,
                    &view.kind,
                    view.style.as_deref(),
                );
                let objects: Vec<_> = view
                    .runs
                    .iter()
                    .filter_map(|run| Some((run.start, object(run.image.as_ref()?, face)?)))
                    .collect();
                (!objects.is_empty()).then_some((view.index, objects))
            })
            .collect();
        let note_look = app.note_look();
        let column = widths.first().copied().unwrap_or(0.0);
        let mut footnotes: HashMap<usize, Vec<(usize, Marginal<'a>)>> = HashMap::new();
        for (index, note) in app.notes() {
            let in_view = viewport.get(index).is_some_and(|view| view.cell.is_none());
            if note.endnote || !in_view {
                continue;
            }
            let band = note_band(app, setter, &note, &note_look.footnote, column);
            footnotes
                .entry(index)
                .or_default()
                .push((note.offset, band));
        }
        let note_heights = footnotes
            .iter()
            .map(|(&index, bands)| {
                let rooms = bands.iter().map(|(at, band)| (*at, band.height)).collect();
                (index, rooms)
            })
            .collect();
        let labels = level_labels(app, viewport, (&faces, &blocks), &mut spacing, &mut indents);
        let frames = paragraph_frames(app, viewport, &mut spacing);
        let insets: HashMap<usize, f64> = frames
            .iter()
            .filter(|(_, frame)| !frame.open_above)
            .map(|(&index, frame)| (index, frame.inset(grind_text::table_look::TOP)))
            .collect();
        let across = widths
            .iter()
            .map(|&width| {
                let looks = crate::faces::TableLooks(&looks);
                flow::across_with(app, viewport, width, &SPACING, &looks)
            })
            .collect();
        let tables = looks
            .iter()
            .map(|(name, look)| {
                use std::hash::{Hash, Hasher};
                let mut hasher = key_hasher();
                format!("{look:?}").hash(&mut hasher);
                (name.clone(), hasher.finish())
            })
            .collect();
        Maps {
            app,
            setter,
            viewport,
            faces,
            blocks,
            spacing,
            breaks,
            insets,
            indents,
            tabs,
            aligns,
            hyphenations,
            objects,
            footnotes,
            note_heights,
            note_look,
            looks,
            tables,
            labels,
            frames,
            widths: widths.to_vec(),
            across,
        }
    }

    /// One [`Column`] per width text is set at.
    fn columns(&self) -> Vec<Column<'_>> {
        self.widths
            .iter()
            .zip(&self.across)
            .map(|(&width, across)| Column {
                insets: &self.insets,
                indents: &self.indents,
                tabs: &self.tabs,
                aligns: &self.aligns,
                hyphenations: &self.hyphenations,
                objects: &self.objects,
                notes: &self.note_heights,
                // The rule, less the space below the page's last note, which is not given
                // (fact 67): the space below a note, the smallest of any, once a page.
                rule: if self.note_heights.is_empty() {
                    0.0
                } else {
                    let below = self
                        .footnotes
                        .values()
                        .flatten()
                        .map(|(_, band)| band.below())
                        .fold(f64::INFINITY, f64::min);
                    (pt(self.note_look.rule.room()) - below).max(0.0)
                },
                looks: &self.looks,
                faces: &self.faces,
                blocks: &self.blocks,
                spacing: &self.spacing,
                breaks: &self.breaks,
                width,
                across,
            })
            .collect()
    }

    /// The face a caption is set in: its paragraph's.
    fn caption_face(&self, view: &BlockView) -> &RoleFace<'_> {
        crate::faces::face_for(
            &self.faces,
            &self.blocks,
            view.index,
            &view.kind,
            view.style.as_deref(),
        )
    }

    fn figure(&self, view: &BlockView, measure: f64) -> Option<Figure> {
        figure(
            view,
            measure,
            self.caption_face(view),
            figure_look(self.app, view),
        )
    }

    /// The block laid out `width` wide, as the flow and the drawing lay it out.
    fn layout(
        &self,
        column: &Column<'_>,
        view: &BlockView,
        width: f64,
    ) -> Option<grind_core::layout::Layout> {
        let face = column.face(view.index, &view.kind, view.style.as_deref());
        let setting = grind_text::Faces::setting(column, view.index);
        self.app
            .layout_block_set(view.index, width as f32, face, &setting)
            .ok()
    }

    /// What of a block laid out as `layout` the cut reads. A block another reads too — a
    /// heading, which names the pages after it, or a table cell or picture, which do not stack
    /// as lines — has none.
    fn shape(&self, view: &BlockView, layout: &grind_core::layout::Layout) -> Option<Shape> {
        use std::hash::{Hash, Hasher};
        if matches!(view.kind, BlockKind::Heading { .. })
            || view.cell.is_some()
            || picture_of(view).is_some()
        {
            return None;
        }
        let mut look = key_hasher();
        (&view.kind, &view.style, &view.label, view.generated).hash(&mut look);
        self.breaks.get(&view.index).hash(&mut look);
        let mut lines = key_hasher();
        for line in layout.lines() {
            (line.top.to_bits(), line.height.to_bits()).hash(&mut lines);
        }
        let opens = self
            .app
            .paragraph_shared(view.index)
            .is_some_and(|resolved| resolved.props.master_page.is_some());
        Some(Shape {
            look: look.finish(),
            lines: lines.finish(),
            counted: view.label.is_some()
                || matches!(view.kind, BlockKind::ListItem { .. })
                || opens,
        })
    }

    /// The headings starting on `page`, for the outline.
    fn headings(
        &self,
        sheet: &Sheet<'_>,
        page: &grind_text::page::Page,
        number: usize,
    ) -> Vec<Heading> {
        page.pieces
            .iter()
            .filter_map(|piece| {
                let view = self.viewport.get(piece.index)?;
                let BlockKind::Heading { level } = view.kind else {
                    return None;
                };
                (piece.lines.start == 0 && self.figure(view, piece.width).is_none()).then(|| {
                    Heading {
                        level,
                        title: view.text.clone(),
                        page: number,
                        y: (sheet.top + piece.top) as f32,
                    }
                })
            })
            .collect()
    }

    /// A hash of everything drawing `page` reads that [`Paper::seed`] does not: its place in the
    /// flow, its number and running heads, and its blocks with their labels, boxes, spacing and
    /// setting — each block counted from the page's first, so a page that blocks added before
    /// it moved keeps its key; [`page_key`] adds where it is.
    fn key(
        &self,
        column: &Column<'_>,
        seed: u64,
        page: &grind_text::page::Page,
        heads: ((usize, usize), &[String]),
    ) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = key_hasher();
        let base = first_block(page);
        seed.hash(&mut hasher);
        (page.slot, page.restart, page.number).hash(&mut hasher);
        heads.hash(&mut hasher);
        for piece in &page.pieces {
            (piece.index - base, piece.lines.clone(), piece.repeat).hash(&mut hasher);
            for value in [piece.top, piece.left, piece.width] {
                drawn(value).hash(&mut hasher);
            }
        }
        for cell in &page.cells {
            (cell.first - base).hash(&mut hasher);
            for value in [cell.top, cell.left, cell.width, cell.height] {
                drawn(value).hash(&mut hasher);
            }
        }
        for index in blocks_of(page) {
            let Some(view) = self.viewport.get(index) else {
                continue;
            };
            fingerprint(view, base, &mut hasher);
            if let Some(cell) = &view.cell {
                self.tables.get(&cell.table).hash(&mut hasher);
            }
            if let Some(label) = self.labels.get(&index) {
                label.x.to_bits().hash(&mut hasher);
                if let Some(bullet) = &label.bullet {
                    // The list style's own picture, which only reading sets: where it is names it.
                    let picture = &bullet.picture;
                    (&picture.mime, &picture.width, &picture.height).hash(&mut hasher);
                    Arc::as_ptr(&picture.data).hash(&mut hasher);
                    (bullet.width.to_bits(), bullet.height.to_bits()).hash(&mut hasher);
                }
            }
            if let Some(frame) = self.frames.get(&index) {
                format!("{frame:?}").hash(&mut hasher);
            }
            if let Some(room) = self.spacing.get(&index) {
                for value in [
                    room.space.above,
                    room.space.below,
                    room.space.left,
                    room.right,
                ] {
                    value.to_bits().hash(&mut hasher);
                }
            }
            grind_text::Faces::setting(column, index).hash(&mut hasher);
        }
        hasher.finish()
    }

    /// Draw `page` on `sheet`: its cells' rules, its blocks, and its header and footer with the
    /// page's number and running heads.
    fn draw(
        &self,
        column: &Column<'_>,
        sheet: &Sheet<'_>,
        page: &grind_text::page::Page,
        ((shown, count), chapter): ((usize, usize), &[String]),
    ) -> Page {
        let (app, setter, viewport) = (self.app, self.setter, self.viewport);
        let (geometry, top, header, footer) =
            (sheet.geometry, sheet.top, &sheet.header, &sheet.footer);
        let (left, margin_top) = (pt(geometry.left), pt(geometry.top));
        let mut ops = Vec::new();
        let mut placed: Vec<LineBox> = Vec::new();
        ops.extend(
            sheet
                .background
                .as_ref()
                .and_then(|background| background_op(background, geometry)),
        );
        // Pictures the text runs through, with the top of the paragraph each floats from and, in
        // a table cell, the cell's box across: drawn over everything once the page's text is.
        type Float<'v> = (usize, &'v grind_text::ImageView, f64, Option<(f64, f64)>);
        let mut floats: Vec<Float<'_>> = Vec::new();
        let mut span: Option<(Caret, Caret)> = None;
        let mut cover = |from: Caret, to: Caret| {
            span = Some(match span {
                Some((start, end)) => (start.min(from), end.max(to)),
                None => (from, to),
            });
        };
        for cell in &page.cells {
            let look = viewport.get(cell.first).and_then(|view| {
                let at = view.cell.as_ref()?;
                let table = self.looks.get(&at.table)?;
                Some((table, table.cell(at.row, at.column)))
            });
            match look {
                // A table whose document styles its cells is drawn as they say — an
                // unstyled cell of it with no rules at all, as Writer draws one.
                Some((table, own)) if !table.cells.is_empty() => {
                    if let Some(own) = own {
                        let last = viewport
                            .get(cell.first)
                            .and_then(|view| view.cell.as_ref())
                            .is_some_and(|at| at.row + at.rows_spanned.max(1) >= table.rows);
                        styled_cell(&mut ops, cell, own, last, left, top);
                    }
                }
                _ => rules(&mut ops, cell, left, top),
            }
        }
        for piece in &page.pieces {
            let Some(view) = viewport.get(piece.index) else {
                continue;
            };
            let origin = (left + piece.left, top + piece.top);
            if let Some(figure) = self.figure(view, piece.width) {
                let length = view.runs.last().map_or(0, grind_text::RunView::end);
                placed.push(figure.line_box(origin, piece.index, length));
                figure.draw(
                    &mut ops,
                    origin,
                    setter,
                    self.caption_face(view),
                    piece.index,
                );
                cover(caret(piece.index, 0), caret(piece.index, length));
                continue;
            }
            let face = column.face(view.index, &view.kind, view.style.as_deref());
            let setting = grind_text::Faces::setting(column, view.index);
            let label = self.labels.get(&view.index);
            let under = ops.len();
            let drawn = lines(
                (&mut ops, &mut placed),
                app,
                view,
                piece,
                face,
                (&setting, label),
                origin,
            );
            if let (Some(frame), Some(drawn)) = (self.frames.get(&view.index), &drawn) {
                let room = self.spacing.get(&view.index).copied().unwrap_or_default();
                let body = (top, top + sheet.height);
                let mut boxed = Vec::new();
                frame.draw(
                    &mut boxed,
                    piece,
                    origin,
                    (drawn.height, drawn.last),
                    room,
                    body,
                );
                ops.splice(under..under, boxed);
            }
            if piece.lines.start == 0 && !piece.repeat {
                let (above, indent) = self
                    .spacing
                    .get(&view.index)
                    .map_or((0.0, 0.0), |room| (room.space.above, room.space.left));
                // In a table cell, across from the cell's box inside its padding, not from the
                // paragraph's indent.
                let cell = view
                    .cell
                    .is_some()
                    .then_some((origin.0 - indent, piece.width + indent));
                floats.extend(
                    view.runs
                        .iter()
                        .filter_map(|run| run.image.as_ref())
                        .filter(|image| image.floating.is_some())
                        .map(|image| (view.index, image, origin.1 - above, cell)),
                );
            }
            if let (Some(drawn), false) = (drawn, piece.repeat) {
                cover(caret(piece.index, drawn.from), caret(piece.index, drawn.to));
            }
        }
        // The page's number within its run, and how many the run has: the whole document's
        // unless a paragraph's style restarted the numbering (`doc/pdf-export.md`).
        let bottom = pt(geometry.height) - pt(geometry.bottom);
        let body_bottom = footer
            .as_ref()
            .map_or(bottom, |footer| bottom - footer.height - footer.spacing);
        let boxes = Boxes {
            paper: (pt(geometry.width), pt(geometry.height)),
            body: (top, body_bottom - top),
        };
        let text_area = (left, pt(geometry.width - geometry.left - geometry.right));
        // The page's footnotes: those cited on the lines it holds, as the cut reserved their
        // room, stacked up from the foot of the body (§5c fact 67).
        let notes: Vec<&Marginal<'_>> = page
            .pieces
            .iter()
            .filter(|piece| !piece.repeat)
            .filter_map(|piece| Some((piece.index, self.footnotes.get(&piece.index)?)))
            .flat_map(|(index, bands)| {
                let placed = &placed;
                bands.iter().filter_map(move |(at, band)| {
                    placed
                        .iter()
                        .any(|line| line.block == index && (line.start..line.end).contains(at))
                        .then_some(band)
                })
            })
            .collect();
        if !notes.is_empty() {
            let rule = &self.note_look.rule;
            // Not the space below the last: its last line ends at the foot of the body.
            let last = notes.last().map_or(0.0, |band| band.below());
            let tall: f64 = notes.iter().map(|band| band.height).sum::<f64>() - last;
            let mut y = body_bottom - tall;
            if rule.drawn {
                let (from, width) = text_area;
                let long = width * rule.share;
                let x = match rule.align.as_str() {
                    "center" => from + (width - long) / 2.0,
                    "right" => from + width - long,
                    _ => from,
                };
                ops.push(Op::Rect {
                    x: x as f32,
                    y: (y - pt(rule.after) - pt(rule.width)) as f32,
                    width: long as f32,
                    height: pt(rule.width) as f32,
                    color: Rgb::parse(&rule.color).unwrap_or(Rgb::BLACK),
                });
            }
            for band in notes {
                band.draw(&mut ops, (left, y), (shown, count, chapter), &boxes);
                y += band.height;
            }
        }
        for (block, image, paragraph, cell) in floats {
            let across = cell.unwrap_or(text_area);
            if let Some(op) = floating_op(image, block, &boxes, across, paragraph) {
                ops.push(op);
            }
        }
        if let Some(header) = header {
            header.draw(
                &mut ops,
                (left, margin_top),
                (shown, count, chapter),
                &boxes,
            );
        }
        if let Some(footer) = footer {
            footer.draw(
                &mut ops,
                (left, bottom - footer.height),
                (shown, count, chapter),
                &boxes,
            );
        }
        Page {
            width: pt(geometry.width) as f32,
            height: pt(geometry.height) as f32,
            ops,
            lines: placed,
            start: span.map(|(start, _)| start),
            end: span.map(|(_, end)| end),
        }
    }
}

/// Every block that prints, as what it is.
fn structure(viewport: &grind_text::Viewport) -> Vec<Element> {
    viewport
        .iter()
        .map(|view| {
            let block = view.index;
            if let Some(cell) = &view.cell {
                return Element::Cell {
                    block,
                    table: cell.table.clone(),
                    row: cell.row,
                    column: cell.column,
                };
            }
            if let Some((_, caption)) = picture_of(view) {
                return Element::Figure {
                    block,
                    alt: caption.map(str::to_owned).filter(|c| !c.trim().is_empty()),
                };
            }
            match view.kind {
                BlockKind::Heading { level } => Element::Heading {
                    block,
                    level,
                    title: view.text.clone(),
                },
                BlockKind::ListItem { depth } => Element::ListItem { block, depth },
                BlockKind::Paragraph => Element::Paragraph { block },
            }
        })
        .collect()
}

/// A hasher for page keys: fast, and the same in every setting of one process.
fn key_hasher() -> impl std::hash::Hasher {
    std::hash::BuildHasher::build_hasher(&foldhash::quality::FixedState::default())
}

/// What of a block a page's drawing reads, into `hasher`. A picture by its length and a sample of
/// its bytes: a picture is put in or taken out, never changed in place.
fn fingerprint(view: &BlockView, base: usize, hasher: &mut impl std::hash::Hasher) {
    use std::hash::Hash;
    (view.index - base, &view.kind, &view.style, &view.text).hash(hasher);
    (
        view.styled,
        &view.marks,
        view.generated,
        &view.cell,
        &view.label,
    )
        .hash(hasher);
    for run in &view.runs {
        (
            run.start, &run.text, &run.props, &run.shown, &run.style, &run.href,
        )
            .hash(hasher);
        if let Some(image) = &run.image {
            (
                &image.mime,
                &image.width,
                &image.height,
                &image.title,
                &image.description,
            )
                .hash(hasher);
            let data = &image.data;
            data.len().hash(hasher);
            data.iter()
                .step_by(data.len() / 256 + 1)
                .for_each(|byte| byte.hash(hasher));
            data[data.len().saturating_sub(64)..].hash(hasher);
        }
    }
}

/// A picture the text runs through, where its graphic style puts it (`doc/odt-format.md` §5c
/// facts 63 and 65): across from the paper's edge or `text_area`'s — a table cell's box, in
/// one — down from the paper's top, the body's or the top of its paragraph, `paragraph` before the paragraph's space above. At the
/// size the document gives it, else its own pixels at 96 to the inch.
fn floating_op(
    image: &grind_text::ImageView,
    block: usize,
    boxes: &Boxes,
    text_area: (f64, f64),
    paragraph: f64,
) -> Option<Op> {
    let (across, down) = image.floating.as_ref()?.placed();
    let length = |value: &Option<String>| value.as_deref().and_then(length_mm).map(pt);
    let (width, height) = if let (Some(w), Some(h)) = (length(&image.width), length(&image.height))
    {
        (w, h)
    } else {
        let (w, h) = grind_text::picture::pixels(&image.data)?;
        (f64::from(w) * 0.75, f64::from(h) * 0.75)
    };
    let scale = pt(1.0);
    let x = across.start(scale, width, (0.0, boxes.paper.0), text_area, text_area);
    let y = down.start(
        scale,
        height,
        (0.0, boxes.paper.1),
        boxes.body,
        (paragraph, 0.0),
    );
    Some(Op::Image {
        x: x as f32,
        y: y as f32,
        width: width as f32,
        height: height as f32,
        mime: image.mime.clone(),
        data: Arc::new(image.data.clone()),
        crop: cut(image.crop.as_ref(), &image.data),
        mark: Mark::Content(block),
    })
}

/// A picture set as a character, as its line sets it: at the size its frame states, reaching
/// above and below the baseline as its seat says (`doc/odt-format.md` §5c facts 29 and 66).
/// `None` for one with no stated size or no measured seat, which stays the placeholder
/// character.
fn object(
    image: &grind_text::ImageView,
    face: &RoleFace<'_>,
) -> Option<grind_core::layout::Object> {
    use grind_text::picture::Seat;
    let length = |value: &Option<String>| value.as_deref().and_then(length_mm).map(pt);
    let (width, height) = (length(&image.width)? as f32, length(&image.height)? as f32);
    let style = face.style(&TextStyle::default());
    let ascent = grind_core::layout::Metrics::ascent(face, &style);
    let descent = grind_core::layout::Metrics::line_height(face, &style) - ascent;
    let above = match image.seat? {
        Seat::Baseline => height,
        Seat::MiddleOfText => (height + ascent - descent) / 2.0,
        Seat::MiddleOfBaseline => height / 2.0,
        Seat::Unmeasured => return None,
    };
    Some(grind_core::layout::Object {
        width,
        above,
        below: height - above,
    })
}

/// What [`Op::Image`] cuts from a picture of `data` its frame crops.
fn cut(crop: Option<&grind_text::picture::Crop>, data: &[u8]) -> [f32; 4] {
    crop.and_then(|crop| crop.fractions(data))
        .map_or([0.0; 4], |fractions| fractions.map(|f| f as f32))
}

/// A page's background picture, behind everything on it: over the whole paper, or at its size
/// with its corner on the paper's bottom-right one (`doc/odt-format.md` §5c fact 44).
/// The other reference points were not measured, and such a picture is not drawn.
fn background_op(
    background: &grind_text::model::PageBackground,
    paper: PageGeometry,
) -> Option<Op> {
    let (width, height) = (pt(paper.width), pt(paper.height));
    let (x, y, w, h) = match &background.placed {
        None => (0.0, 0.0, width, height),
        Some((w, h, at)) if at == "bottom-right" => {
            let (w, h) = (pt(*w), pt(*h));
            (width - w, height - h, w, h)
        }
        Some(_) => return None,
    };
    Some(Op::Image {
        x: x as f32,
        y: y as f32,
        width: w as f32,
        height: h as f32,
        mime: background.mime.clone(),
        data: background.data.clone(),
        crop: [0.0; 4],
        mark: Mark::Decoration,
    })
}

/// One kind of page the document is set on: a master page, resolved.
#[derive(Clone, Debug, PartialEq)]
struct Kind {
    /// The master's name, or `None` for the document's own default page.
    name: Option<String>,
    /// Whether this is the first page of a run opening on the master, which wears the master's
    /// first-page header and footer (`doc/odt-format.md` §5c facts 24 and 37).
    first: bool,
    /// Whether it is a left page (fact 35).
    left: bool,
    next: usize,
    /// [`grind_text::page::Slot`]'s: the master by number, this kind as a plain page and as a
    /// left one, and the only hand its pages may be on.
    master: usize,
    plain: usize,
    left_twin: usize,
    hand: Option<grind_text::page::Hand>,
}

/// A kind of page, laid out: its geometry, its header and footer, and what is left for the body.
struct Sheet<'a> {
    geometry: PageGeometry,
    background: Option<grind_text::model::PageBackground>,
    header: Option<Marginal<'a>>,
    footer: Option<Marginal<'a>>,
    /// Where the body starts down the page, and how tall it is — in points.
    top: f64,
    height: f64,
}

impl<'a> Sheet<'a> {
    fn lay_out(
        app: &'a App,
        setter: &'a Typesetter,
        kind: &Kind,
        default: PageGeometry,
        options: &Options,
    ) -> Self {
        let master = kind.name.as_deref().and_then(|name| app.master(name));
        let geometry = options
            .paper
            .or_else(|| master.as_ref().and_then(|m| m.geometry))
            .unwrap_or(default);
        let mut geometry = geometry;
        let (header, footer) = match &master {
            Some(master) => match (&master.first, &master.left) {
                (Some(first), _) if kind.first => first.clone(),
                (_, Some(left)) if kind.left => left.clone(),
                _ => (master.header.clone(), master.footer.clone()),
            },
            None => app.marginals(),
        };
        if kind.left
            && master
                .as_ref()
                .is_some_and(|m| m.usage == grind_text::PageUsage::Mirrored)
        {
            std::mem::swap(&mut geometry.left, &mut geometry.right);
        }
        let width = pt(geometry.text_width());
        let trimmed = |m: grind_text::marginal::Marginal| {
            let least = pt(m.min_height) - pt(m.spacing);
            Marginal::lay_out(app, setter, &m, width).trimmed(least)
        };
        let header = header.map(trimmed);
        let footer = footer.map(trimmed);
        let room = |m: &Option<Marginal<'_>>| m.as_ref().map_or(0.0, |m| m.height + m.spacing);
        let top = pt(geometry.top) + room(&header);
        let height = (pt(geometry.text_height()) - room(&header) - room(&footer)).max(1.0);
        Sheet {
            geometry,
            background: master.as_ref().and_then(|m| m.background.clone()),
            header,
            footer,
            top,
            height,
        }
    }
}

/// The kinds of page the document uses — kind `0` is the one its first page is set on — and the
/// blocks that open a page of a kind of their own.
///
/// A master page is named by the paragraph that starts it (`style:master-page-name`, `doc/odt-format.md`
/// §5c fact 17), and the pages after it are set on its `style:next-style-name`: a letter
/// series is a letter's first paragraph, over and over, and each letter gets its letterhead.
fn page_kinds(app: &App, viewport: &grind_text::Viewport) -> (Vec<Kind>, grind_text::page::Starts) {
    let mut kinds: Vec<Kind> = Vec::new();
    let kind_of = |kinds: &mut Vec<Kind>, name: Option<&str>, first: bool, left: bool| -> usize {
        if let Some(at) = kinds.iter().position(|kind| {
            kind.name.as_deref() == name && kind.first == first && kind.left == left
        }) {
            at
        } else {
            kinds.push(Kind {
                name: name.map(str::to_owned),
                first,
                left,
                next: 0,
                master: 0,
                plain: 0,
                left_twin: 0,
                hand: None,
            });
            kinds.len() - 1
        }
    };
    // A run opens on the master's first page where it has one.
    let has_first = |name: Option<&str>| {
        name.and_then(|name| app.master(name))
            .is_some_and(|master| master.first.is_some())
    };
    let opening_of =
        |kinds: &mut Vec<Kind>, name: Option<&str>| kind_of(kinds, name, has_first(name), false);
    let first = viewport
        .iter()
        .next()
        .and_then(|view| app.paragraph_shared(view.index)?.props.master_page.clone());
    let rest = app.rest_master();
    // The page the document opens on: what its first paragraph names, else the body's master.
    let opening = first.or_else(|| rest.clone());
    opening_of(&mut kinds, opening.as_deref());
    let mut starts = grind_text::page::Starts::new();
    for view in viewport.iter().filter(|view| view.cell.is_none()) {
        let Some(resolved) = app.paragraph_shared(view.index) else {
            continue;
        };
        let props = &resolved.props;
        let Some(name) = props.master_page.clone() else {
            continue;
        };
        let restart = props
            .page_number
            .as_deref()
            .and_then(|n| n.trim().parse::<u32>().ok());
        let at = opening_of(&mut kinds, Some(&name));
        starts.insert(view.index, (at, restart));
    }
    // Each kind's successor — its master's `next-style-name`, else its own ordinary page, a first
    // page only when the master changes (fact 37) — and its plain and left twins. These may be
    // kinds nothing else names, so this grows the list it walks.
    let mut masters: Vec<Option<String>> = Vec::new();
    let mut at = 0;
    while at < kinds.len() {
        let name = kinds[at].name.clone();
        let master = name.as_deref().and_then(|name| app.master(name));
        let next = match master.as_ref().and_then(|m| m.next.clone()) {
            Some(next)
                if next.as_str() != name.as_deref().unwrap_or_default()
                    && app.master(&next).is_some() =>
            {
                kind_of(&mut kinds, Some(&next), has_first(Some(&next)), false)
            }
            _ => kind_of(&mut kinds, name.as_deref(), false, false),
        };
        let (first, left) = (kinds[at].first, kinds[at].left);
        let plain = kind_of(&mut kinds, name.as_deref(), false, left);
        let differs = master
            .as_ref()
            .is_some_and(|m| m.left.is_some() || m.usage == grind_text::PageUsage::Mirrored);
        let left_twin = if differs {
            kind_of(&mut kinds, name.as_deref(), first, true)
        } else {
            at
        };
        let hand = master.as_ref().and_then(|m| match m.usage {
            grind_text::PageUsage::Left => Some(grind_text::page::Hand::Left),
            grind_text::PageUsage::Right => Some(grind_text::page::Hand::Right),
            _ => None,
        });
        let number = if let Some(number) = masters.iter().position(|m| *m == name) {
            number
        } else {
            masters.push(name.clone());
            masters.len() - 1
        };
        let kind = &mut kinds[at];
        (kind.next, kind.plain, kind.left_twin) = (next, plain, left_twin);
        (kind.master, kind.hand) = (number, hand);
        at += 1;
    }
    (kinds, starts)
}

/// The heading each outline level shows on each page — the last of that level or above begun
/// on it or before it (`doc/odt-format.md` §5c fact 43) — its line breaks as spaces — going on
/// from `current`, what the page before showed.
fn chapters(
    mut current: Vec<String>,
    pages: &[grind_text::page::Page],
    viewport: &grind_text::Viewport,
) -> Vec<Vec<String>> {
    current.resize(10, String::new());
    pages
        .iter()
        .map(|page| {
            for piece in page
                .pieces
                .iter()
                .filter(|p| p.lines.start == 0 && !p.repeat)
            {
                if let Some(view) = viewport.get(piece.index)
                    && let BlockKind::Heading { level } = view.kind
                    && view.cell.is_none()
                {
                    let name = view.text.replace('\n', " ");
                    for shown in current.iter_mut().skip((level as usize).max(1) - 1) {
                        shown.clone_from(&name);
                    }
                }
            }
            current.clone()
        })
        .collect()
}

/// Each page's number and the last number of its run: the whole document is one run unless a
/// paragraph's style restarted the numbering (`style:page-number`), in which case the pages
/// from there to the next restart are — so "page 1 of 2" is a letter's own, in a series.
fn numbering(pages: &[grind_text::page::Page]) -> Vec<(usize, usize)> {
    // From the back, each run's last number carried to its first page.
    let mut shown = vec![(0, 0); pages.len()];
    let mut last = 0;
    for (at, page) in pages.iter().enumerate().rev() {
        if at + 1 == pages.len() || pages[at + 1].restart.is_some() {
            last = page.number as usize;
        }
        shown[at] = (page.number as usize, last);
    }
    shown
}

/// A face for every block whose paragraph style says something about its text: all four properties
/// when the document declares the block's style, and only the default style's family when it does
/// not — such a block keeps the screen's size and weight for its role, the better guess for a
/// heading whose style is missing.
fn block_faces<'a>(
    app: &App,
    setter: &'a Typesetter,
    viewport: &grind_text::Viewport,
) -> std::collections::HashMap<usize, RoleFace<'a>> {
    viewport
        .iter()
        .filter_map(|view| {
            let resolved = app.paragraph_shared(view.index)?;
            let props = &resolved.props;
            let stated = if resolved.declared {
                TextStyle {
                    font_family: props.font_family.clone(),
                    font_size: props.font_size.clone(),
                    font_weight: props.font_weight.clone(),
                    font_style: props.font_style.clone(),
                    letter_spacing: props.letter_spacing.clone(),
                    letter_kerning: props.letter_kerning.clone(),
                    text_position: None,
                    font_variant: props.font_variant.clone(),
                    text_transform: props.text_transform.clone(),
                    mid_word: false,
                }
            } else {
                TextStyle {
                    font_family: props.font_family.clone(),
                    letter_spacing: props.letter_spacing.clone(),
                    letter_kerning: props.letter_kerning.clone(),
                    ..TextStyle::default()
                }
            };
            // A declared style is the whole answer, so its block is set as body text under it:
            // a heading style that states no size or weight is not given the screen's.
            let role = if resolved.declared {
                grind_text::look::Role::Body
            } else {
                grind_text::look::Role::of(&view.kind, view.style.as_deref())
            };
            let line = crate::faces::LineRule::parse(props.line_height.as_deref());
            (resolved.declared || stated != TextStyle::default() || line.is_some()).then(|| {
                let face = RoleFace::stating(setter, role, stated).with_line(line);
                (view.index, face)
            })
        })
        .collect()
}

/// The space above and below every block whose paragraph style the document declares, in points —
/// added, as Writer adds them (`doc/odt-format.md` §5c, fact 5), and taken out between
/// neighbours of one style under contextual spacing (fact 31). A margin with no length (a
/// percentage of an unread parent) counts as none.
fn block_spacing(
    app: &App,
    viewport: &grind_text::Viewport,
) -> std::collections::HashMap<usize, crate::faces::Room> {
    let points = |value: &Option<String>| value.as_deref().and_then(length_mm).map_or(0.0, pt);
    let mut spacing: HashMap<usize, crate::faces::Room> = HashMap::new();
    // The block before: what contextual spacing compares.
    struct Before {
        index: usize,
        named: Option<String>,
        contextual: bool,
        cell: Option<(String, u32, u32)>,
    }
    let mut before: Option<Before> = None;
    for view in viewport.iter() {
        let Some(resolved) = app.paragraph_shared(view.index).filter(|r| r.declared) else {
            before = None;
            continue;
        };
        // A block in a table too: Writer spaces a cell's paragraphs by their own margins,
        // inside the cell's padding (`doc/odt-format.md` §5c fact 18).
        let props = &resolved.props;
        let mut room = crate::faces::Room {
            space: grind_text::flow::Space {
                above: points(&props.margin_top),
                below: points(&props.margin_bottom),
                left: points(&props.margin_left),
            },
            right: points(&props.margin_right),
        };
        let cell = view
            .cell
            .as_ref()
            .map(|cell| (cell.table.clone(), cell.row, cell.column));
        let contextual = props.contextual();
        if let Some(before) = &before
            && before.index + 1 == view.index
            && before.cell == cell
            && before.named.is_some()
            && before.named == resolved.named
            && before.contextual
        {
            if let Some(above) = spacing.get_mut(&before.index) {
                above.space.below = 0.0;
            }
            if contextual {
                room.space.above = 0.0;
            }
        }
        before = Some(Before {
            index: view.index,
            named: resolved.named.clone(),
            contextual,
            cell,
        });
        spacing.insert(view.index, room);
    }
    spacing
}

/// Each boxed paragraph's frame, by block index, with its borders and padding added to the room
/// it takes (`doc/odt-format.md` §5c fact 30). A paragraph joins the box of the one before it
/// when they sit together — both outside a table or in the same cell — and their borders, padding
/// and side margins are equal, unless either says `style:join-border="false"`; no border or
/// padding is taken between them.
fn paragraph_frames(
    app: &App,
    viewport: &grind_text::Viewport,
    spacing: &mut HashMap<usize, crate::faces::Room>,
) -> HashMap<usize, Frame> {
    let resolved: Vec<Option<std::sync::Arc<grind_text::paragraph::Resolved>>> = viewport
        .iter()
        .map(|view| app.paragraph_shared(view.index).filter(|r| r.declared))
        .collect();
    let props: Vec<Option<&grind_text::paragraph::ParagraphProps>> = resolved
        .iter()
        .map(|resolved| resolved.as_deref().map(|resolved| &resolved.props))
        .collect();
    let views: Vec<&BlockView> = viewport.iter().collect();
    let looks: Vec<_> = props
        .iter()
        .map(|props| props.and_then(grind_text::paragraph::ParagraphProps::look))
        .collect();
    let joined = |at: usize| -> bool {
        let (Some(a), Some(b)) = (&props[at], &props[at + 1]) else {
            return false;
        };
        let (Some(first), Some(second)) = (&looks[at], &looks[at + 1]) else {
            return false;
        };
        let (va, vb) = (views[at], views[at + 1]);
        let cell = |view: &BlockView| {
            view.cell
                .as_ref()
                .map(|cell| (cell.table.clone(), cell.row, cell.column))
        };
        let length = |value: &Option<String>| value.as_deref().and_then(length_mm).unwrap_or(0.0);
        vb.index == va.index + 1
            && cell(va) == cell(vb)
            && a.joins()
            && b.joins()
            && first.border == second.border
            && first.padding == second.padding
            && length(&a.margin_left) == length(&b.margin_left)
            && length(&a.margin_right) == length(&b.margin_right)
    };
    let mut frames = HashMap::new();
    for (at, look) in looks.iter().enumerate() {
        let Some(look) = look else {
            continue;
        };
        let Some(room) = spacing.get_mut(&views[at].index) else {
            continue;
        };
        let frame = Frame {
            look: look.clone(),
            open_above: at > 0 && joined(at - 1),
            open_below: at + 1 < views.len() && joined(at),
        };
        let inset = |side: usize| frame.inset(side);
        use grind_text::table_look::{BOTTOM, LEFT, RIGHT, TOP};
        room.space.left += inset(LEFT);
        room.right += inset(RIGHT);
        if !frame.open_above {
            room.space.above += inset(TOP);
        }
        if !frame.open_below {
            room.space.below += inset(BOTTOM);
        }
        frames.insert(views[at].index, frame);
    }
    frames
}

/// A paragraph's box: its look, and whether it runs on into the paragraph above or below.
#[derive(Debug)]
struct Frame {
    look: grind_text::table_look::CellLook,
    open_above: bool,
    open_below: bool,
}

impl Frame {
    /// The room `side`'s border and padding take, in points.
    fn inset(&self, side: usize) -> f64 {
        self.look.edge(side).map_or(0.0, |edge| edge.thickness) + pt(self.look.padding[side])
    }

    /// Draw the box round one piece of its paragraph, whose lines start at `(x, y)` and are
    /// `height` tall: the fill over the paragraph's own box, then each closed side's border
    /// inside its outer edge. A side open to a joined neighbour runs through the margins between
    /// them, and stops at the body's edge.
    // ponytail: a box split by a page break is left open at the break; how Writer draws a box
    // there was not measured.
    fn draw(
        &self,
        ops: &mut Vec<Op>,
        piece: &Piece,
        (x, y): (f64, f64),
        (height, last): (f64, bool),
        room: crate::faces::Room,
        (body_top, body_bottom): (f64, f64),
    ) {
        use grind_text::table_look::{BOTTOM, LEFT, RIGHT, TOP};
        let first = piece.lines.start == 0;
        let (x0, x1) = (x - self.inset(LEFT), x + piece.width + self.inset(RIGHT));
        let closed_top = first && !self.open_above;
        let closed_bottom = last && !self.open_below;
        let y0 = match (first, self.open_above) {
            (false, _) => y,
            (true, false) => y - self.inset(TOP),
            (true, true) => (y - room.space.above).max(body_top),
        };
        let y1 = match (last, self.open_below) {
            (false, _) => y + height,
            (true, false) => y + height + self.inset(BOTTOM),
            (true, true) => (y + height + room.space.below).min(body_bottom),
        };
        if let Some(fill) = self.look.background.as_deref().and_then(Rgb::parse) {
            let top = if first { y - self.inset(TOP) } else { y };
            let bottom = if last {
                y + height + self.inset(BOTTOM)
            } else {
                y + height
            };
            ops.push(Op::Rect {
                x: x0 as f32,
                y: top as f32,
                width: (x1 - x0) as f32,
                height: (bottom - top) as f32,
                color: fill,
            });
        }
        for (side, closed) in [
            (TOP, closed_top),
            (RIGHT, true),
            (BOTTOM, closed_bottom),
            (LEFT, true),
        ] {
            let Some(edge) = self.look.edge(side).filter(|_| closed) else {
                continue;
            };
            let color = Rgb::parse(&edge.color).unwrap_or(Rgb::BLACK);
            for &(offset, width) in &edge.lines {
                let at = offset + width / 2.0;
                let (from, to) = match side {
                    TOP => ((x0, y0 + at), (x1, y0 + at)),
                    BOTTOM => ((x0, y1 - at), (x1, y1 - at)),
                    LEFT => ((x0 + at, y0), (x0 + at, y1)),
                    _ => ((x1 - at, y0), (x1 - at, y1)),
                };
                ops.push(Op::Line {
                    from: (from.0 as f32, from.1 as f32),
                    to: (to.0 as f32, to.1 as f32),
                    width: width as f32,
                    color,
                });
            }
        }
    }
}

/// Where each numbered heading's number and each list item's label go, by block index — across
/// from the block's own left edge, in points — after setting the block's left margin and
/// first-line indent as its level places them (`doc/odt-format.md` §5c facts 20 and 22,
/// [`App::level_indent`]). The label sits at the margin plus the first-line indent; the text
/// follows at the level's tab stop — or, past it, at the next default stop counted from the
/// margin — or after a space, or straight after, as `text:label-followed-by` says; a level in the
/// older mode adds its room to the paragraph's own margins instead (fact 60). A block the
/// level gives no label — an unnumbered heading, a list header, an item's second paragraph —
/// starts at the margin. A block whose style is not declared keeps the page's own margin, and
/// its label moves with it; a list naming no style keeps the shell's bullet where it was.
fn level_labels(
    app: &App,
    viewport: &grind_text::Viewport,
    (faces, blocks): (&[RoleFace<'_>], &HashMap<usize, RoleFace<'_>>),
    spacing: &mut HashMap<usize, crate::faces::Room>,
    indents: &mut HashMap<usize, f32>,
) -> HashMap<usize, Label> {
    let mut labels = HashMap::new();
    for view in viewport.iter() {
        let label = match view.kind {
            BlockKind::Heading { .. } if view.number().is_some() || view.unnumbered() => {
                view.number()
            }
            BlockKind::ListItem { .. } if view.label.is_some() => view.mark(),
            _ => continue,
        };
        let Some(level) = app.level_indent(view.index) else {
            continue;
        };
        let length = |value: &Option<String>| value.as_deref().and_then(length_mm).map(pt);
        // The older mode adds the level's room to the paragraph's own margins (§5c fact 60):
        // the margin is where the later lines start, the label `width` before it.
        let older = level.width_and_position.as_ref().map(|place| {
            let own = app.paragraph_shared(view.index);
            let props = own.as_ref().map(|resolved| &resolved.props);
            let of = |value: Option<&Option<String>>| value.and_then(length).unwrap_or(0.0);
            let width = of(Some(&place.min_label_width));
            (
                of(props.map(|p| &p.margin_left)) + of(Some(&place.space_before)) + width,
                of(props.map(|p| &p.text_indent)),
                width,
                of(Some(&place.min_label_distance)),
            )
        });
        let (margin, indent) = match older {
            Some((margin, indent, ..)) => (margin, indent),
            None => (
                length(&level.margin_left).unwrap_or(0.0),
                length(&level.text_indent).unwrap_or(0.0),
            ),
        };
        // The flow sets a list item in by its depth before a style's own margin; the level's
        // margin replaces both.
        let role = SPACING.indent_of(&view.kind);
        let applied = match spacing.get_mut(&view.index) {
            Some(room) => {
                room.space.left = margin - role;
                margin
            }
            None => role,
        };
        let first = match label {
            None if older.is_some() => margin + indent - applied,
            None => margin - applied,
            Some(label) => {
                let face = crate::faces::face_for(
                    faces,
                    blocks,
                    view.index,
                    &view.kind,
                    view.style.as_deref(),
                );
                let style = face.style(&TextStyle::default());
                let setter = face.setter();
                let wide = |text: &str| -> f64 {
                    setter
                        .shape(text, &style)
                        .glyphs
                        .iter()
                        .map(|glyph| f64::from(glyph.x_advance))
                        .sum()
                };
                // A picture bullet takes its own width, and makes the line taller where it is
                // taller than the text's middle leaves room for (§5c fact 41).
                let length_pt = |value: &Option<String>| length(value).unwrap_or(0.0);
                let bullet = app.level_picture(view.index).map(|picture| Bullet {
                    width: length_pt(&picture.width),
                    height: length_pt(&picture.height),
                    picture,
                });
                if let (Some(bullet), Some(room)) = (&bullet, spacing.get_mut(&view.index)) {
                    let ascent = f64::from(grind_core::layout::Metrics::ascent(face, &style));
                    let line = f64::from(grind_core::layout::Metrics::line_height(face, &style));
                    let middle = (ascent - (line - ascent)) / 2.0;
                    room.space.above += (middle + bullet.height / 2.0 - ascent).max(0.0);
                    room.space.below += (bullet.height / 2.0 - middle - (line - ascent)).max(0.0);
                }
                let label_wide = bullet
                    .as_ref()
                    .map_or_else(|| wide(label), |bullet| bullet.width);
                // Placed at the indent by the level's alignment, and its tab to the stop past it,
                // else the margin past it, else a default stop (§5c fact 48).
                let start = match older {
                    Some((_, _, width, _)) => margin - width + indent,
                    None => {
                        margin + indent
                            - match level.label_align.as_deref() {
                                Some("end") => label_wide,
                                Some("center") => label_wide / 2.0,
                                _ => 0.0,
                            }
                    }
                };
                let end = start + label_wide;
                let text = match (older, level.followed_by.as_deref()) {
                    (Some((_, _, width, distance)), _) => (start + width).max(end + distance),
                    (None, followed_by) => match followed_by {
                        Some("space") => end + wide(" "),
                        Some("nothing") => end,
                        _ => match length(&level.tab) {
                            Some(tab) if tab > end => tab,
                            _ if margin > end => margin,
                            _ => {
                                let interval = app
                                    .paragraph_shared(view.index)
                                    .map(|resolved| f64::from(resolved.props.tabs().interval))
                                    .filter(|interval| *interval > 0.0)
                                    .unwrap_or(pt(12.5));
                                let past = ((end - margin) / interval).floor() + 1.0;
                                margin + interval * past.max(0.0)
                            }
                        },
                    },
                };
                labels.insert(
                    view.index,
                    Label {
                        x: start - applied,
                        bullet,
                    },
                );
                text - applied
            }
        };
        if first == 0.0 {
            indents.remove(&view.index)
        } else {
            indents.insert(view.index, first as f32)
        };
    }
    labels
}

/// Where a block's label goes, across from its own left edge in points, and the picture drawn
/// there instead of text when its level is a picture bullet.
#[derive(Debug)]
struct Label {
    x: f64,
    bullet: Option<Bullet>,
}

/// A picture bullet at the size its level states, in points.
// ponytail: the room it adds is added above and below the paragraph, which is exact for the
// one-line paragraph it usually heads; a longer one gets the extra under its last line.
#[derive(Debug)]
struct Bullet {
    picture: grind_text::numbering::LevelPicture,
    width: f64,
    height: f64,
}

/// Two paragraphs one after the other, outside tables, kept the larger of the first's space below
/// and the second's above apart rather than both (`doc/odt-format.md` §5c fact 46): the first's
/// below shrinks by the second's above.
fn keep_the_larger_space(
    viewport: &grind_text::Viewport,
    spacing: &mut HashMap<usize, crate::faces::Room>,
) {
    let views: Vec<&BlockView> = viewport.iter().collect();
    for [first, second] in views.array_windows() {
        if first.cell.is_some() || second.cell.is_some() || second.index != first.index + 1 {
            continue;
        }
        let Some(above) = spacing.get(&second.index).map(|room| room.space.above) else {
            continue;
        };
        if let Some(room) = spacing.get_mut(&first.index) {
            room.space.below = (room.space.below - above).max(0.0);
        }
    }
}

/// Every block whose paragraph style says something about page breaks around it: a break before or
/// after (`page`; `even-page`/`odd-page` as `page`, since no blank pages are printed), and
/// `fo:keep-with-next` (`always`, or `auto` to let even a heading go).
fn block_breaks(
    app: &App,
    viewport: &grind_text::Viewport,
) -> std::collections::HashMap<usize, grind_text::page::Breaks> {
    let page = |value: &Option<String>| {
        matches!(value.as_deref(), Some("page" | "even-page" | "odd-page"))
    };
    viewport
        .iter()
        .filter(|view| view.cell.is_none())
        .filter_map(|view| {
            let resolved = app.paragraph_shared(view.index)?;
            let props = &resolved.props;
            let breaks = grind_text::page::Breaks {
                page_before: page(&props.break_before),
                page_after: page(&props.break_after),
                keep_with_next: match props.keep_with_next.as_deref() {
                    Some("always") => Some(true),
                    Some("auto") => Some(false),
                    _ => None,
                },
            };
            (breaks != grind_text::page::Breaks::default()).then_some((view.index, breaks))
        })
        .collect()
}

fn caret(block: usize, offset: usize) -> Caret {
    Caret { block, offset }
}

/// A styled cell: its fill under everything, and each of its four borders where Writer puts
/// it (`doc/odt-format.md` §5c fact 18): down a column's edge, centred on it; across a row's
/// top, in the band the cell's padding left room for above its content; under the table's last
/// row, in the band at the bottom of the cell; and under any other row, in the band at the top
/// of the next. A `double` border is its two lines ([`grind_text::table_look::Edge`]).
fn styled_cell(
    ops: &mut Vec<Op>,
    cell: &CellBox,
    look: &grind_text::table_look::CellLook,
    last_row: bool,
    left: f64,
    top: f64,
) {
    use grind_text::table_look::{BOTTOM, LEFT, RIGHT, TOP};
    let (x0, y0) = (left + cell.left, top + cell.top);
    let (x1, y1) = (x0 + cell.width, y0 + cell.height);
    if let Some(fill) = look.background.as_deref().and_then(Rgb::parse) {
        ops.push(Op::Rect {
            x: x0 as f32,
            y: y0 as f32,
            width: (x1 - x0) as f32,
            height: (y1 - y0) as f32,
            color: fill,
        });
    }
    let edges = [TOP, RIGHT, BOTTOM, LEFT].map(|side| look.edge(side));
    let thickness = |side: usize| edges[side].as_ref().map_or(0.0, |edge| edge.thickness);
    // Where the second line of a double starts, from its band's outside edge: an inner line
    // meets the inner lines across it, an outer one runs out to the outer ones, so a double
    // border is two closed frames as Writer draws it rather than lines crossing at a corner.
    let inner_from = |side: usize| match &edges[side] {
        Some(edge) if edge.lines.len() > 1 => edge.lines[1].0,
        _ => 0.0,
    };
    // The bottom band is inside the cell on the table's last row and the next row's otherwise.
    let bottom = if last_row { y1 } else { y1 + thickness(BOTTOM) };
    for (side, edge) in [TOP, RIGHT, BOTTOM, LEFT].into_iter().zip(&edges) {
        let Some(edge) = edge else {
            continue;
        };
        let color = Rgb::parse(&edge.color).unwrap_or(Rgb::BLACK);
        // The band's outside edge, and which way is inward from it.
        let (outside, inward) = match side {
            TOP => (y0, 1.0),
            BOTTOM => (bottom, -1.0),
            LEFT => (x0 - edge.thickness / 2.0, 1.0),
            _ => (x1 + edge.thickness / 2.0, -1.0),
        };
        for (k, &(offset, width)) in edge.lines.iter().enumerate() {
            let at = outside + inward * (offset + width / 2.0);
            let line = match side {
                TOP | BOTTOM => {
                    let (left, right) = (x0 - thickness(LEFT) / 2.0, x1 + thickness(RIGHT) / 2.0);
                    match k {
                        0 => ((left, at), (right, at)),
                        _ => (
                            (left + inner_from(LEFT), at),
                            (right - inner_from(RIGHT), at),
                        ),
                    }
                }
                _ => match k {
                    0 => ((at, y0), (at, bottom)),
                    _ => (
                        (at, y0 + inner_from(TOP)),
                        (at, bottom - inner_from(BOTTOM)),
                    ),
                },
            };
            ops.push(Op::Line {
                from: (line.0.0 as f32, line.0.1 as f32),
                to: (line.1.0 as f32, line.1.1 as f32),
                width: width as f32,
                color,
            });
        }
    }
}

/// How wide a line is for aligning it: up to its last visible character — a space a line was
/// broken at hangs past the measure and is not aligned — except on a paragraph's **last** line,
/// whose trailing spaces Writer counts: a centred `X ` sits half a space left of a centred `X`,
/// and a right-aligned one a whole space (`doc/odt-format.md` §5c fact 19).
fn aligned_width(
    layout: &grind_core::layout::Layout,
    line: &grind_core::layout::Line,
    chars: &[char],
    visible_end: usize,
    last: bool,
) -> f32 {
    let end = if last {
        (visible_end..line.end)
            .take_while(|&i| chars.get(i) == Some(&' '))
            .last()
            .map_or(visible_end, |i| i + 1)
    } else {
        visible_end
    };
    if end == line.end {
        line.width
    } else {
        layout.x_at(end)
    }
}

/// The four sides of a table cell.
fn rules(ops: &mut Vec<Op>, cell: &CellBox, left: f64, top: f64) {
    let (x0, y0) = ((left + cell.left) as f32, (top + cell.top) as f32);
    let (x1, y1) = (x0 + cell.width as f32, y0 + cell.height as f32);
    for (from, to) in [
        ((x0, y0), (x1, y0)),
        ((x1, y0), (x1, y1)),
        ((x0, y1), (x1, y1)),
        ((x0, y0), (x0, y1)),
    ] {
        ops.push(Op::Line {
            from,
            to,
            width: RULE,
            color: Rgb::BLACK,
        });
    }
}

/// What [`lines`] drew of a block: the character offsets the piece runs between, how tall its
/// lines are, and whether they end the block.
struct Drawn {
    from: usize,
    to: usize,
    height: f64,
    last: bool,
}

/// One piece of a block: its lines, each cut where its formatting changes, and the bullet in
/// front of a list item's first line.
fn lines(
    (ops, boxes): (&mut Vec<Op>, &mut Vec<LineBox>),
    app: &App,
    view: &BlockView,
    piece: &Piece,
    face: &RoleFace<'_>,
    (setting, label): (&grind_core::layout::Setting, Option<&Label>),
    (left, top): (f64, f64),
) -> Option<Drawn> {
    let layout = app
        .layout_block_set(view.index, piece.width as f32, face, setting)
        .ok()?;
    let (first_indent, tabs) = (setting.first, &setting.tabs);
    let first = layout.lines().get(piece.lines.start)?;
    let last = layout.lines().get(piece.lines.end.checked_sub(1)?)?;
    let span = (first.start, last.end);
    let first_top = first.top;
    let setter = face.setter();
    let chars: Vec<char> = view.text.chars().collect();
    let count = layout.lines().len();
    // The paragraph style's ink, under every run that states none.
    let ink = app
        .paragraph_shared(view.index)
        .and_then(|resolved| resolved.props.color.clone())
        .as_deref()
        .and_then(Rgb::parse)
        .unwrap_or(Rgb::BLACK);
    for at in piece.lines.clone() {
        let Some(line) = layout.lines().get(at) else {
            continue;
        };
        // The line's visible end: the spaces before it are the ones justification widened.
        let visible_end = (line.start..line.end)
            .rev()
            .find(|&i| chars.get(i).is_some_and(|c| !c.is_whitespace()))
            .map_or(line.start, |i| i + 1);
        let line_top = top as f32 + (line.top - first_top);
        let baseline = line_top + layout.baseline_of(line);
        // A heading row drawn again on a later page is not somewhere a caret can be.
        if !piece.repeat {
            boxes.push(LineBox {
                block: view.index,
                start: line.start,
                end: line.end,
                top: line_top,
                height: line.height,
                baseline,
                xs: (line.start..=line.end)
                    .map(|offset| left as f32 + layout.x_on(at, offset))
                    .collect(),
                last: at + 1 == count,
            });
        }
        // A numbered heading's number or a list item's label, where its level puts it
        // ([`level_labels`]).
        if at == 0
            && let Some(Label {
                x: dx,
                bullet: Some(bullet),
            }) = label
        {
            let style = face.style(&TextStyle::default());
            let ascent = grind_core::layout::Metrics::ascent(face, &style);
            let descent = grind_core::layout::Metrics::line_height(face, &style) - ascent;
            let middle = baseline - (ascent - descent) / 2.0;
            ops.push(Op::Image {
                x: (left + dx) as f32,
                y: middle - bullet.height as f32 / 2.0,
                width: bullet.width as f32,
                height: bullet.height as f32,
                mime: bullet.picture.mime.clone(),
                data: bullet.picture.data.clone(),
                crop: [0.0; 4],
                mark: match (piece.repeat, &view.kind) {
                    (true, _) => Mark::Decoration,
                    (false, _) => Mark::Label(view.index),
                },
            });
        } else if at == 0
            && let (Some(number), Some(&Label { x: dx, .. })) =
                (view.number().or(view.mark()), label)
        {
            let style = face.style(&TextStyle::default());
            let (id, size) = setter.face_of(&style);
            ops.push(Op::Text {
                x: (left + dx) as f32,
                y: baseline,
                face: id,
                size,
                glyphs: setter.shape(number, &style).glyphs,
                text: number.to_owned(),
                color: ink,
                // A heading's number is part of the heading as a reader hears it; a list
                // item's label is the item's label.
                mark: match (piece.repeat, &view.kind) {
                    (true, _) => Mark::Decoration,
                    (false, BlockKind::ListItem { .. }) => Mark::Label(view.index),
                    (false, _) => Mark::Content(view.index),
                },
            });
        }
        // A list item's label — its number, or its style's bullet (`BlockView::mark`) — in
        // front of its first line, and never into the text: a label wider than the room a
        // bullet leaves ends a little before it.
        if at == 0
            && label.is_none()
            && let Some(mark) = view.mark()
        {
            let style = face.style(&TextStyle::default());
            let glyphs = setter.shape(mark, &style).glyphs;
            let wide: f64 = glyphs.iter().map(|g| f64::from(g.x_advance)).sum();
            let gap = f64::from(setter.face_of(&style).1) * 0.3;
            ops.push(Op::Text {
                x: (left - (SPACING.indent * 2.0 / 3.0).max(wide + gap)) as f32,
                y: baseline,
                face: setter.face_of(&style).0,
                size: setter.face_of(&style).1,
                glyphs,
                text: mark.to_owned(),
                color: Rgb::BLACK,
                mark: if piece.repeat {
                    Mark::Decoration
                } else {
                    Mark::Label(view.index)
                },
            });
        }
        // Each tab's gap filled with its stop's leader — a table of contents' dots — from just
        // after the text before it to just before the text after it (fact 16).
        for i in (line.start..line.end).filter(|&i| chars.get(i) == Some(&'\t')) {
            let origin = if at == 0 { first_indent } else { 0.0 };
            let (from, to) = (layout.x_at(i), layout.x_at(i + 1));
            let to = if i + 1 == line.end {
                line.shift + line.width
            } else {
                to
            };
            let Some(leader) = tabs.next(origin + from).and_then(|stop| stop.leader) else {
                continue;
            };
            let style = face.style(&TextStyle::default());
            let mark = leader.to_string();
            let one = setter.shape(&mark, &style);
            let step: f32 = one.glyphs.iter().map(|g| g.x_advance).sum();
            let room = to - from - step;
            if step <= 0.0 || room < step {
                continue;
            }
            let count = (room / step).floor() as usize;
            let dots = mark.repeat(count);
            ops.push(Op::Text {
                x: left as f32 + to - step * (count as f32 + 0.5),
                y: baseline,
                face: one.face,
                size: one.size,
                glyphs: setter.shape(&dots, &style).glyphs,
                text: dots,
                color: ink,
                mark: Mark::Decoration,
            });
        }
        // Its pictures set as characters, each where its seat puts it (§5c fact 66).
        for &(offset, object) in setting
            .objects
            .iter()
            .filter(|(offset, _)| (line.start..line.end).contains(offset))
        {
            let Some(image) = view
                .runs
                .iter()
                .find(|run| run.start == offset)
                .and_then(|run| run.image.as_ref())
            else {
                continue;
            };
            ops.push(Op::Image {
                x: left as f32 + layout.x_at(offset),
                y: baseline - object.above,
                width: object.width,
                height: object.above + object.below,
                mime: image.mime.clone(),
                data: Arc::new(image.data.clone()),
                crop: cut(image.crop.as_ref(), &image.data),
                mark: Mark::Content(view.index),
            });
        }
        for cut in paint::pieces(&view.runs, line.start, line.end) {
            if setting
                .objects
                .iter()
                .any(|(offset, _)| *offset == cut.start)
                && cut.text == "\u{fffc}"
            {
                continue;
            }
            for (start, text) in paint::drawable(cut.start, cut.text) {
                if text.is_empty() {
                    continue;
                }
                let style = face.style(&grind_core::style::TextStyle {
                    mid_word: start
                        .checked_sub(1)
                        .and_then(|before| chars.get(before))
                        .is_some_and(|c| !c.is_whitespace()),
                    ..cut.props.metrics()
                });
                let mut shaped = setter.shape(text, &style);
                let x = left as f32 + layout.x_at(start);
                if line.stretch > 0.0 {
                    // Every interior space this piece holds is drawn `extra` wider.
                    for glyph in &mut shaped.glyphs {
                        let here = start + text[..glyph.text.start].chars().count();
                        if &text[glyph.text.clone()] == " " && here < visible_end {
                            glyph.x_advance += line.stretch;
                        }
                    }
                }
                if line.letters > 0.0 {
                    // A word spread over the line: every letter but the last is drawn wider.
                    let glyphs = &mut shaped.glyphs;
                    for at in 0..glyphs.len() {
                        let cluster = glyphs[at].text.clone();
                        let closes = glyphs.get(at + 1).is_none_or(|next| next.text != cluster);
                        let here = start + text[..cluster.start].chars().count();
                        if closes && here + 1 < visible_end {
                            glyphs[at].x_advance += line.letters;
                        }
                    }
                }
                let run: f32 = shaped.glyphs.iter().map(|g| g.x_advance).sum();
                // What is drawn ends where the layout put the caret after it: shaping a piece
                // alone and measuring its run whole must agree (`doc/page-model.md` PM1).
                #[cfg(test)]
                {
                    let end = start + text.chars().count();
                    if end < visible_end {
                        let caret = left as f32 + layout.x_on(at, end);
                        assert!(
                            (x + run - caret).abs() < 0.01,
                            "{text:?} drawn to {} with its caret at {caret}",
                            x + run
                        );
                    }
                }
                // A link with no colour of its own is drawn as Writer draws an unstyled one,
                // navy and underlined (`doc/odt-format.md` §5c, fact 12) — but not an index's
                // entry, whose link Writer styles `Index Link`, plain (fact 16).
                let link = !view.generated
                    && view
                        .runs
                        .iter()
                        .any(|run| run.href.is_some() && run.start <= start && start < run.end());
                let color = match (cut.props.color.as_deref().and_then(Rgb::parse), link) {
                    (Some(color), _) => color,
                    (None, true) => LINK,
                    (None, false) => ink,
                };
                if let Some(fill) = cut.props.background.as_deref().and_then(Rgb::parse) {
                    ops.push(Op::Rect {
                        x,
                        y: line_top,
                        width: run,
                        height: line.height,
                        color: fill,
                    });
                }
                let size = shaped.size;
                // A raised or lowered run sits off the line's baseline (fact 56).
                let baseline = baseline - setter.rise(&style);
                ops.push(Op::Text {
                    x,
                    y: baseline,
                    face: shaped.face,
                    size,
                    glyphs: shaped.glyphs,
                    text: text.to_owned(),
                    color,
                    // A heading row repeated on a later page is drawn, not read again.
                    mark: if piece.repeat {
                        Mark::Decoration
                    } else {
                        Mark::Content(view.index)
                    },
                });
                // Where an underline and a strike sit, as fractions of the size: close to what
                // the bundled faces' own `post` and `OS/2` tables say, and the same for any face.
                let underline = match (&cut.props.underline, link) {
                    (None, true) => Some("solid".to_owned()),
                    (stated, _) => stated.clone(),
                };
                for (on, drop) in [(&underline, 0.12), (&cut.props.line_through, -0.3)] {
                    if on.as_deref().is_some_and(|value| value != "none") {
                        let y = baseline + size * drop;
                        ops.push(Op::Line {
                            from: (x, y),
                            to: (x + run, y),
                            width: size * 0.06,
                            color,
                        });
                    }
                }
            }
        }
        // A line broken at a soft hyphen shows it, in its run's face and colour; not text a
        // reader copies, which is the word whole.
        if line.hyphen > 0.0
            && let Some(cut) = paint::pieces(&view.runs, line.end - 1, line.end).first()
        {
            let style = face.style(&cut.props.metrics());
            let shaped = setter.shape("-", &style);
            ops.push(Op::Text {
                x: left as f32 + layout.x_on(at, line.end),
                y: baseline,
                face: shaped.face,
                size: shaped.size,
                glyphs: shaped.glyphs,
                text: "-".to_owned(),
                color: cut
                    .props
                    .color
                    .as_deref()
                    .and_then(Rgb::parse)
                    .unwrap_or(ink),
                mark: Mark::Decoration,
            });
        }
    }
    Some(Drawn {
        from: span.0,
        to: span.1,
        height: f64::from(last.top + last.height - first_top),
        last: piece.lines.end >= count,
    })
}

/// A picture block on paper: the picture at the size the document gives it, fitted to the
/// measure, and its caption's lines under it.
struct Figure {
    width: f64,
    /// The measure its caption is set at — the column's, whatever the picture's width.
    measure: f64,
    picture: f64,
    mime: String,
    data: Arc<Vec<u8>>,
    crop: [f32; 4],
    caption: Option<String>,
    height: f64,
    /// Its paragraph's alignment, which places the picture and each caption line, and the
    /// space between them (`doc/odt-format.md` §5c fact 40).
    align: Align,
    gap: f64,
}

/// A block's figure, when it is a picture whose size the document states. Paper is physical, so
/// `svg:width` and `svg:height` are exactly right here, where a screen ignores them. A picture
/// with no stated size is measured as text (ponytail: a natural size needs decoding its pixels,
/// the shells' `Decoder`, which this crate lacks).
fn figure(
    view: &BlockView,
    measure: f64,
    body: &RoleFace<'_>,
    (align, gap): (Align, f64),
) -> Option<Figure> {
    let (image, caption) = picture_of(view)?;
    let length = |value: &Option<String>| value.as_deref().and_then(length_mm).map(pt);
    // The size the document gives it, or — for a picture that states none, as one imported
    // from Markdown does — its own pixels at 96 to the inch, the size a browser would draw it.
    let (w, h) = if let (Some(w), Some(h)) = (length(&image.width), length(&image.height)) {
        (w, h)
    } else {
        let (w, h) = grind_text::picture::pixels(&image.data)?;
        (f64::from(w) * 0.75, f64::from(h) * 0.75)
    };
    let (w, h) = (w.max(1.0), h.max(1.0));
    let width = w.min(measure.max(1.0));
    let picture = h * width / w;
    let caption = caption.map(str::to_owned).filter(|c| !c.trim().is_empty());
    let below = caption.as_deref().map_or(0.0, |text| {
        gap + f64::from(caption_layout(text, measure, body).height())
    });
    Some(Figure {
        width,
        measure: measure.max(1.0),
        picture,
        mime: image.mime.clone(),
        data: Arc::new(image.data.clone()),
        crop: cut(image.crop.as_ref(), &image.data),
        caption,
        height: picture + below,
        align,
        gap,
    })
}

/// How a picture block's paragraph sets it: its alignment, and the space between picture and
/// caption — the style's own below and above, as Writer's picture and caption paragraphs share
/// the frame's (`doc/odt-format.md` §5c fact 40). A block whose style is not declared gets the
/// start and [`CAPTION_GAP`].
// ponytail: the frame's inner paragraphs' own styles are not kept; theirs is assumed.
fn figure_look(app: &App, view: &BlockView) -> (Align, f64) {
    let Some(resolved) = app.paragraph_shared(view.index).filter(|r| r.declared) else {
        return (Align::parse(None), CAPTION_GAP);
    };
    let props = &resolved.props;
    let points = |value: &Option<String>| value.as_deref().and_then(length_mm).map_or(0.0, pt);
    (
        Align::parse(props.text_align.as_deref()),
        points(&props.margin_bottom) + points(&props.margin_top),
    )
}

fn caption_layout(text: &str, measure: f64, body: &RoleFace<'_>) -> grind_core::layout::Layout {
    let style = TextStyle::default();
    wrap(
        &[Fragment {
            text,
            style: &style,
        }],
        measure as f32,
        body,
    )
}

impl Figure {
    /// The picture as somewhere a caret can be: before it at its left edge, after it — and
    /// anywhere in its caption — at its right.
    // ponytail: a caption's characters have no places of their own, so a click in one lands
    // after the picture. Place its lines like a paragraph's when a caption is edited in a page.
    fn line_box(&self, (x, y): (f64, f64), block: usize, length: usize) -> LineBox {
        let (offset, _) = self
            .align
            .fit(self.measure as f32, self.width as f32, 0, true);
        let left = x as f32 + offset;
        let mut xs = vec![left + self.width as f32; length + 1];
        xs[0] = left;
        LineBox {
            block,
            start: 0,
            end: length,
            top: y as f32,
            height: self.picture as f32,
            baseline: (y + self.picture) as f32,
            xs,
            last: true,
        }
    }

    fn draw(
        &self,
        ops: &mut Vec<Op>,
        (x, y): (f64, f64),
        setter: &Typesetter,
        body: &RoleFace<'_>,
        block: usize,
    ) {
        let offset =
            |width: f64| f64::from(self.align.fit(self.measure as f32, width as f32, 0, true).0);
        ops.push(Op::Image {
            x: (x + offset(self.width)) as f32,
            y: y as f32,
            width: self.width as f32,
            height: self.picture as f32,
            mime: self.mime.clone(),
            data: self.data.clone(),
            crop: self.crop,
            mark: Mark::Content(block),
        });
        let Some(caption) = &self.caption else {
            return;
        };
        let layout = caption_layout(caption, self.measure, body);
        let chars: Vec<char> = caption.chars().collect();
        let style = body.style(&TextStyle::default());
        let (face, size) = setter.face_of(&style);
        for line in layout.lines() {
            let text: String = chars[line.start..line.end].iter().collect();
            let text = text.trim_end().to_owned();
            let top = (y + self.picture + self.gap) as f32 + line.top;
            let glyphs = setter.shape(&text, &style).glyphs;
            let wide: f32 = glyphs.iter().map(|g| g.x_advance).sum();
            ops.push(Op::Text {
                x: (x + offset(f64::from(wide))) as f32,
                y: top + layout.baseline(),
                face,
                size,
                glyphs,
                text,
                color: Rgb::BLACK,
                mark: Mark::Content(block),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fonts::Fonts;
    use crate::ops::{Op, Rgb};
    use grind_text::{BlockKind, Caret, CharStyle};
    use std::fmt::Write as _;

    fn setter() -> Typesetter {
        Typesetter::new(Fonts::bundled())
    }

    fn doc(blocks: &[(BlockKind, &str)]) -> App {
        let app = App::new();
        for (at, (kind, text)) in blocks.iter().enumerate() {
            app.insert(at, kind.clone(), text).unwrap();
        }
        app.delete(blocks.len()..blocks.len() + 1).unwrap();
        app
    }

    /// A letter typed into a paragraph that keeps its lines: the pages are not cut again, only
    /// the one holding it is drawn, and the result is what setting it from nothing gives. Words
    /// that add lines, and Enter, are cut again from near the edit, to the same result.
    #[test]
    fn an_edit_that_keeps_a_paragraph_s_lines_draws_only_its_page() {
        let paragraph = "All work and no play makes a dull document. ".repeat(12);
        let app = doc(&vec![(BlockKind::Paragraph, paragraph.as_str()); 40]);
        let (setter, options) = (setter(), Options::default());
        let before = typeset(&app, &setter, &options);
        assert!(before.pages.len() > 4);
        app.insert_text(
            Caret {
                block: 20,
                offset: 0,
            },
            "x",
        )
        .unwrap();
        let after = typeset_after(&app, &setter, &options, Some(&before));
        let cut = |set: &Document| Arc::clone(&set.cut.as_ref().unwrap().pages);
        assert!(Arc::ptr_eq(&cut(&before), &cut(&after)), "cut again");
        let redrawn = (before.pages.iter().zip(&after.pages))
            .filter(|(a, b)| !Arc::ptr_eq(a, b))
            .count();
        assert_eq!(redrawn, 1);
        let fresh = typeset(&app, &setter, &options);
        assert_eq!(after.pages, fresh.pages);
        assert_eq!(after.keys, fresh.keys);

        app.insert_text(
            Caret {
                block: 20,
                offset: 0,
            },
            &"longer ".repeat(20),
        )
        .unwrap();
        assert!(
            touch_up(&app, &setter, &options, &after).is_none(),
            "a line more"
        );
        let longer = recut(&app, &setter, &options, &after).expect("cut again near the edit");
        let fresh = typeset(&app, &setter, &options);
        assert_eq!(longer.pages, fresh.pages);
        assert_eq!(longer.keys, fresh.keys);
        assert_eq!(longer.cut, fresh.cut, "cut as if from nothing");
        assert_eq!(longer.outline, fresh.outline);

        // Enter: a paragraph more, and every page after it moved on by one block.
        app.split_block(Caret {
            block: 10,
            offset: 30,
        })
        .unwrap();
        let split = recut(&app, &setter, &options, &longer).expect("cut again near Enter");
        let fresh = typeset(&app, &setter, &options);
        assert_eq!(split.pages, fresh.pages);
        assert_eq!(split.keys, fresh.keys);
        assert_eq!(split.structure, fresh.structure);
        assert_eq!(split.cut, fresh.cut);
    }

    /// Every text op on a page as `(x, y, text)`.
    fn texts(page: &crate::ops::Page) -> Vec<(f32, f32, String)> {
        page.ops
            .iter()
            .filter_map(|op| match op {
                Op::Text { x, y, text, .. } => Some((*x, *y, text.clone())),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_word_broken_at_its_soft_hyphen_shows_a_hyphen_inside_the_margin() {
        let words = "Donau\u{AD}dampf\u{AD}schiffahrts\u{AD}gesellschafts\u{AD}kapitän ".repeat(20);
        let app = doc(&[(BlockKind::Paragraph, words.trim_end())]);
        let t = setter();
        let typeset = typeset(&app, &t, &Options::default());
        let page = &typeset.pages[0];
        let right = page.width - 56.6929;
        let hyphens: Vec<f32> = page
            .ops
            .iter()
            .filter_map(|op| match op {
                Op::Text {
                    x,
                    text,
                    glyphs,
                    mark: crate::ops::Mark::Decoration,
                    ..
                } if text == "-" => Some(x + glyphs.iter().map(|g| g.x_advance).sum::<f32>()),
                _ => None,
            })
            .collect();
        assert!(!hyphens.is_empty(), "some line breaks inside a word");
        for end in hyphens {
            assert!(end <= right + 0.01, "a hyphen ends at {end}, past {right}");
        }
        // Mid-line a soft hyphen is nothing: no line shows more hyphens than it breaks at.
        let lines = page.lines.len();
        let shown = page
            .ops
            .iter()
            .filter(|op| matches!(op, Op::Text { text, .. } if text == "-"))
            .count();
        assert!(shown < lines);
    }

    #[test]
    fn a_short_document_is_one_a4_page_set_inside_its_margins() {
        let app = doc(&[(BlockKind::Paragraph, "Hello, paper.")]);
        let t = setter();
        let typeset = typeset(&app, &t, &Options::default());
        assert_eq!(typeset.pages.len(), 1);
        let page = &typeset.pages[0];
        assert!((page.width - 595.2756).abs() < 1e-3 && (page.height - 841.8898).abs() < 1e-3);
        let texts = texts(page);
        assert_eq!(texts.len(), 1);
        let (x, y, text) = &texts[0];
        assert_eq!(text, "Hello, paper.");
        assert!((x - 56.6929).abs() < 1e-3, "the left margin, 20 mm: {x}");
        let ascent = grind_core::layout::Metrics::ascent(
            &t,
            &grind_core::style::TextStyle {
                font_size: Some("12pt".into()),
                ..Default::default()
            },
        );
        assert!(
            (y - (56.6929 + ascent)).abs() < 1e-3,
            "the first baseline: {y}"
        );
    }

    #[test]
    fn the_paper_asked_for_wins_over_the_documents() {
        let app = doc(&[(BlockKind::Paragraph, "x")]);
        let options = Options {
            paper: PageGeometry::paper("a5-landscape"),
        };
        let page = &typeset(&app, &setter(), &options).pages[0];
        assert!((page.width - pt(210.0) as f32).abs() < 1e-3);
        assert!((page.height - pt(148.0) as f32).abs() < 1e-3);
    }

    #[test]
    fn every_character_is_printed_once_however_many_pages_it_takes() {
        let paragraph = "The quick brown fox jumps over the lazy dog, again and again. ".repeat(12);
        let blocks: Vec<(BlockKind, &str)> = (0..30)
            .map(|_| (BlockKind::Paragraph, paragraph.as_str()))
            .collect();
        let app = doc(&blocks);
        let typeset = typeset(&app, &setter(), &Options::default());
        assert!(typeset.pages.len() > 2, "{} pages", typeset.pages.len());
        let printed: String = typeset
            .pages
            .iter()
            .flat_map(|page| texts(page))
            .map(|(_, _, t)| t)
            .collect();
        let expected: String = (0..30).map(|_| paragraph.as_str()).collect();
        assert_eq!(printed.replace(' ', ""), expected.replace(' ', ""));
        for page in &typeset.pages {
            for (_, y, _) in texts(page) {
                assert!(
                    y > 56.0 && y < page.height - 56.0,
                    "inside the margins: {y}"
                );
            }
        }
    }

    #[test]
    fn a_bold_run_is_set_in_the_bold_face_and_a_coloured_one_in_its_colour() {
        let app = doc(&[(BlockKind::Paragraph, "plain loud")]);
        let style = CharStyle {
            font_weight: Some("bold".into()),
            color: Some("#c00000".into()),
            background: Some("#ffff00".into()),
            underline: Some("solid".into()),
            ..CharStyle::default()
        };
        app.set_char_style(
            Caret {
                block: 0,
                offset: 6,
            },
            Caret {
                block: 0,
                offset: 10,
            },
            &style,
        )
        .unwrap();
        let t = setter();
        let page = &typeset(&app, &t, &Options::default()).pages[0];
        let loud = page.ops.iter().find_map(|op| match op {
            Op::Text {
                text, face, color, ..
            } if text == "loud" => Some((*face, *color)),
            _ => None,
        });
        let (face, color) = loud.expect("the bold run is its own piece");
        assert!(t.fonts().face(face).bold);
        assert_eq!(color, Rgb(0xc0, 0, 0));
        assert!(page.ops.iter().any(|op| matches!(
            op,
            Op::Rect {
                color: Rgb(255, 255, 0),
                ..
            }
        )));
        assert!(page.ops.iter().any(|op| matches!(
            op,
            Op::Line {
                color: Rgb(0xc0, 0, 0),
                ..
            }
        )));
    }

    #[test]
    fn a_list_item_is_indented_and_has_its_bullet_before_it() {
        let app = doc(&[
            (BlockKind::Paragraph, "para"),
            (BlockKind::ListItem { depth: 1 }, "item"),
        ]);
        let page = &typeset(&app, &setter(), &Options::default()).pages[0];
        let texts = texts(page);
        let para = texts.iter().find(|t| t.2 == "para").unwrap();
        let item = texts.iter().find(|t| t.2 == "item").unwrap();
        let bullet = texts.iter().find(|t| t.2 == "\u{2022}").unwrap();
        assert!((item.0 - para.0 - 18.0).abs() < 1e-3, "one list level in");
        assert!(bullet.0 < item.0 && bullet.0 >= para.0);
        assert_eq!(bullet.1, item.1, "on the item's baseline");
    }

    #[test]
    fn every_heading_is_in_the_outline_with_the_page_it_landed_on() {
        let long = "words ".repeat(400);
        let app = doc(&[
            (BlockKind::Heading { level: 1 }, "One"),
            (BlockKind::Paragraph, &long),
            (BlockKind::Paragraph, &long),
            (BlockKind::Heading { level: 2 }, "Two"),
        ]);
        let typeset = typeset(&app, &setter(), &Options::default());
        let outline: Vec<(u32, &str, usize)> = typeset
            .outline
            .iter()
            .map(|h| (h.level, h.title.as_str(), h.page))
            .collect();
        assert_eq!(outline[0], (1, "One", 0));
        assert_eq!(outline[1].0, 2);
        assert_eq!(outline[1].2, typeset.pages.len() - 1);
    }

    #[test]
    fn a_table_is_drawn_with_its_rules() {
        let app = doc(&[(BlockKind::Paragraph, "before")]);
        app.insert_table(1, 2, 2, None).unwrap();
        app.insert_text(
            Caret {
                block: 1,
                offset: 0,
            },
            "cell",
        )
        .unwrap();
        let page = &typeset(&app, &setter(), &Options::default()).pages[0];
        let rules = page
            .ops
            .iter()
            .filter(|op| matches!(op, Op::Line { .. }))
            .count();
        assert_eq!(rules, 16, "four sides of four cells");
        assert!(texts(page).iter().any(|t| t.2 == "cell"));
    }

    /// A caption is set at the column's measure, which is what its height was taken at: under a
    /// picture narrower than a word it was drawn a word to a line, over whatever came next.
    #[test]
    fn a_narrow_pictures_caption_is_set_at_the_measure() {
        let app = doc(&[(BlockKind::Paragraph, "")]);
        app.insert_image(
            Caret {
                block: 0,
                offset: 0,
            },
            "image/png".into(),
            b"\x89PNG\r\n\x1a\n".to_vec(),
            Some("2mm".into()),
            Some("3mm".into()),
        )
        .unwrap();
        app.insert_text(
            Caret {
                block: 0,
                offset: 1,
            },
            "a caption of eight short words in it",
        )
        .unwrap();
        let page = &typeset(&app, &setter(), &Options::default()).pages[0];
        let lines = texts(page).len();
        assert_eq!(lines, 1, "{:?}", texts(page));
    }

    /// `doc/odt-format.md` §5c fact 41: a picture bullet is drawn at its stated size where the
    /// label starts, centred on the text's middle, the text straight after it, and its line made
    /// taller by it.
    #[test]
    fn a_picture_bullet_is_drawn_in_front_of_its_item() {
        let item = |list: &str| {
            format!(
                r#"<text:list text:style-name="{list}"><text:list-item><text:p text:style-name="P">Note</text:p></text:list-item></text:list>"#
            )
        };
        let level = |name: &str, size: &str| {
            format!(
                r#"<text:list-style style:name="{name}"><text:list-level-style-image text:level="1"><office:binary-data>iVBORw0KGgo=</office:binary-data><style:list-level-properties text:list-level-position-and-space-mode="label-alignment" style:vertical-pos="middle" style:vertical-rel="char" fo:width="{size}" fo:height="{size}"><style:list-level-label-alignment text:label-followed-by="nothing"/></style:list-level-properties></text:list-level-style-image></text:list-style>"#
            )
        };
        let bytes = format!(
            r#"<office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" office:mimetype="application/vnd.oasis.opendocument.text">
            <office:styles><style:style style:name="P" style:family="paragraph"/>{big}{small}</office:styles>
            <office:body><office:text><text:p text:style-name="P">above</text:p>{one}<text:p text:style-name="P">between</text:p>{two}<text:p text:style-name="P">after</text:p></office:text></office:body></office:document>"#,
            big = level("Big", "0.688cm"),
            small = level("Small", "0.2cm"),
            one = item("Big"),
            two = item("Small"),
        );
        let app = App::new();
        app.open_bytes("bullet.fodt", bytes.as_bytes()).unwrap();
        let page = &typeset(&app, &setter(), &Options::default()).pages[0];
        let images: Vec<(f32, f32, f32, f32)> = page
            .ops
            .iter()
            .filter_map(|op| match op {
                Op::Image {
                    x,
                    y,
                    width,
                    height,
                    ..
                } => Some((*x, *y, *width, *height)),
                _ => None,
            })
            .collect();
        assert_eq!(images.len(), 2, "{images:?}");
        let cm = (72.0 / 2.54) as f32;
        let (x, y, width, height) = images[0];
        assert!((width - 0.688 * cm).abs() < 0.01 && (height - 0.688 * cm).abs() < 0.01);
        let texts = texts(page);
        let at = |word: &str| texts.iter().find(|t| t.2 == word).unwrap().clone();
        let notes: Vec<_> = texts.iter().filter(|t| t.2 == "Note").collect();
        assert!(
            (x - at("above").0).abs() < 0.01,
            "the picture where the label starts"
        );
        assert!(
            (notes[0].0 - (x + width)).abs() < 0.01,
            "the text straight after it"
        );
        let middle = y + height / 2.0;
        let above_baseline = notes[0].1 - middle;
        assert!(
            (3.5..5.0).contains(&above_baseline),
            "centred {above_baseline} above"
        );
        let pitch = |a: &str, b: f32| b - at(a).1;
        let plain = notes[1].1 - at("between").1;
        assert!(
            pitch("above", notes[0].1) > plain + 2.5,
            "the big one moves its text down"
        );
        assert!(
            at("between").1 - notes[0].1 > plain + 2.0,
            "and the line after it"
        );
        assert!(
            (at("after").1 - notes[1].1 - plain).abs() < 0.01,
            "a small one does not"
        );
    }

    /// `doc/odt-format.md` §5c fact 40: Writer's captioned picture, in a centred style with
    /// space below, is centred with its caption, the style's space between them.
    #[test]
    fn a_captioned_picture_is_centred_with_its_caption_as_its_style_says() {
        let bytes = r#"<office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0" xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0" office:mimetype="application/vnd.oasis.opendocument.text">
            <office:styles><style:style style:name="Figure" style:family="paragraph"><style:paragraph-properties fo:text-align="center" fo:margin-top="0cm" fo:margin-bottom="0.5cm"/></style:style></office:styles>
            <office:automatic-styles><style:page-layout style:name="pm"><style:page-layout-properties fo:page-width="21cm" fo:page-height="29.7cm" fo:margin-top="2cm" fo:margin-bottom="2cm" fo:margin-left="2cm" fo:margin-right="2cm"/></style:page-layout></office:automatic-styles>
            <office:master-styles><style:master-page style:name="Standard" style:page-layout-name="pm"/></office:master-styles>
            <office:body><office:text>
              <text:p text:style-name="Figure"><draw:frame text:anchor-type="as-char"><draw:text-box fo:min-width="0.041cm">
                <text:p text:style-name="Figure"><draw:frame text:anchor-type="as-char" svg:width="4cm" svg:height="2cm"><draw:image draw:mime-type="image/png"><office:binary-data>iVBORw0KGgo=</office:binary-data></draw:image></draw:frame></text:p>
                <text:p text:style-name="Figure">Figure 1: Caption</text:p>
              </draw:text-box></draw:frame></text:p>
            </office:text></office:body></office:document>"#;
        let app = App::new();
        app.open_bytes("fig.fodt", bytes.as_bytes()).unwrap();
        let page = &typeset(&app, &setter(), &Options::default()).pages[0];
        let cm = 72.0 / 2.54;
        let centre = (10.5 * cm) as f32;
        let (x, y, width, height) = page
            .ops
            .iter()
            .find_map(|op| match op {
                Op::Image {
                    x,
                    y,
                    width,
                    height,
                    ..
                } => Some((*x, *y, *width, *height)),
                _ => None,
            })
            .unwrap();
        assert!((x + width / 2.0 - centre).abs() < 0.01, "picture at {x}");
        let caption = page
            .ops
            .iter()
            .find_map(|op| match op {
                Op::Text { x, y, glyphs, .. } => {
                    Some((*x, *y, glyphs.iter().map(|g| g.x_advance).sum::<f32>()))
                }
                _ => None,
            })
            .unwrap();
        assert!(
            (caption.0 + caption.2 / 2.0 - centre).abs() < 0.01,
            "caption at {}",
            caption.0
        );
        // The caption's baseline sits a line's ascent below the gap: under 1.5 lines of 12 pt.
        let gap = caption.1 - (y + height);
        assert!(
            gap > (0.5 * cm) as f32 && gap < (0.5 * cm) as f32 + 14.0,
            "caption baseline {gap} below the picture"
        );
    }

    #[test]
    fn a_picture_prints_at_the_size_the_document_gives_it_fitted_to_the_measure() {
        let app = doc(&[(BlockKind::Paragraph, ""), (BlockKind::Paragraph, "")]);
        let png = b"\x89PNG\r\n\x1a\n".to_vec();
        let at = |block| Caret { block, offset: 0 };
        app.insert_image(
            at(0),
            "image/png".into(),
            png.clone(),
            Some("5cm".into()),
            Some("2.5cm".into()),
        )
        .unwrap();
        app.insert_image(
            at(1),
            "image/png".into(),
            png,
            Some("40cm".into()),
            Some("20cm".into()),
        )
        .unwrap();
        let page = &typeset(&app, &setter(), &Options::default()).pages[0];
        let boxes: Vec<(f32, f32)> = page
            .ops
            .iter()
            .filter_map(|op| match op {
                Op::Image { width, height, .. } => Some((*width, *height)),
                _ => None,
            })
            .collect();
        assert_eq!(boxes.len(), 2);
        assert!(
            (boxes[0].0 - pt(50.0) as f32).abs() < 1e-3
                && (boxes[0].1 - pt(25.0) as f32).abs() < 1e-3
        );
        let measure = pt(170.0) as f32;
        assert!((boxes[1].0 - measure).abs() < 1e-3 && (boxes[1].1 - measure / 2.0).abs() < 1e-3);
    }

    #[test]
    fn each_page_knows_the_first_and_last_character_on_it() {
        let paragraph = "The quick brown fox jumps over the lazy dog. ".repeat(60);
        let blocks: Vec<(BlockKind, &str)> = (0..6)
            .map(|_| (BlockKind::Paragraph, paragraph.as_str()))
            .collect();
        let app = doc(&blocks);
        let typeset = typeset(&app, &setter(), &Options::default());
        assert!(typeset.pages.len() >= 2);
        let first = &typeset.pages[0];
        assert_eq!(
            first.start,
            Some(Caret {
                block: 0,
                offset: 0
            })
        );
        // Each page starts exactly where the one before it ended.
        for [prev, next] in typeset.pages.array_windows() {
            assert_eq!(prev.end, next.start);
        }
        let last = typeset.pages.last().unwrap();
        let length = paragraph.chars().count();
        assert_eq!(
            last.end,
            Some(Caret {
                block: 5,
                offset: length
            })
        );
        // Some page starts inside a paragraph rather than at one.
        assert!(
            typeset
                .pages
                .iter()
                .any(|p| p.start.is_some_and(|c| c.offset > 0))
        );
    }

    #[test]
    fn every_block_is_in_the_structure_and_its_text_is_marked_as_its_own() {
        let app = doc(&[
            (BlockKind::Heading { level: 2 }, "Two"),
            (BlockKind::Paragraph, "para"),
            (BlockKind::ListItem { depth: 1 }, "item"),
        ]);
        app.insert_table(3, 1, 2, Some("T".into())).unwrap();
        let typeset = typeset(&app, &setter(), &Options::default());
        use crate::ops::{Element, Mark};
        assert_eq!(
            typeset.structure,
            vec![
                Element::Heading {
                    block: 0,
                    level: 2,
                    title: "Two".into()
                },
                Element::Paragraph { block: 1 },
                Element::ListItem { block: 2, depth: 1 },
                Element::Cell {
                    block: 3,
                    table: "T".into(),
                    row: 0,
                    column: 0
                },
                Element::Cell {
                    block: 4,
                    table: "T".into(),
                    row: 0,
                    column: 1
                },
            ]
        );
        let marks: Vec<(String, Mark)> = typeset.pages[0]
            .ops
            .iter()
            .filter_map(|op| match op {
                Op::Text { text, mark, .. } => Some((text.clone(), *mark)),
                _ => None,
            })
            .collect();
        assert!(marks.contains(&("para".into(), Mark::Content(1))));
        assert!(marks.contains(&("\u{2022}".into(), Mark::Label(2))));
    }

    /// `doc/odt-format.md` §5c fact 3: a paragraph that does not fit leaves a lone line on the
    /// next page when the document states no widows, and two when it states two.
    #[test]
    fn widows_are_the_documents_own_and_none_when_it_states_none() {
        let lines_on = |widows: Option<&str>| {
            let props = widows.map_or(String::new(), |w| {
                format!(r#"<office:styles><style:default-style style:family="paragraph"><style:paragraph-properties fo:widows="{w}" fo:orphans="{w}"/></style:default-style></office:styles>"#)
            });
            // A page with room for exactly four lines of 12 pt Liberation Serif (13.8 pt each),
            // and a paragraph of five.
            let height = 4.0 * 13.8 * 25.4 / 72.0 + 0.5;
            let bytes = format!(
                r#"<office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" office:mimetype="application/vnd.oasis.opendocument.text">{props}<office:automatic-styles><style:page-layout style:name="pm"><style:page-layout-properties fo:page-width="10cm" fo:page-height="{height}mm"/></style:page-layout></office:automatic-styles><office:master-styles><style:master-page style:name="Standard" style:page-layout-name="pm"/></office:master-styles><office:body><office:text><text:p>a<text:line-break/>b<text:line-break/>c<text:line-break/>d<text:line-break/>e</text:p></office:text></office:body></office:document>"#
            );
            let app = App::new();
            app.open_bytes("w.fodt", bytes.as_bytes()).unwrap();
            let doc = typeset(&app, &setter(), &Options::default());
            doc.pages.iter().map(|p| texts(p).len()).collect::<Vec<_>>()
        };
        assert_eq!(lines_on(None), vec![4, 1]);
        assert_eq!(lines_on(Some("2")), vec![3, 2]);
    }

    /// `doc/odt-format.md` §5c fact 55: the measuring document's sentence on its 5 cm measure,
    /// broken where Writer broke it — with hyphenation on, and not at all with it off.
    #[test]
    #[cfg(feature = "bundled-hyphenation")]
    fn a_hyphenated_paragraph_breaks_its_words_where_writer_does() {
        let lines = |hyphenate: &str| {
            let bytes = format!(
                r#"<office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" office:mimetype="application/vnd.oasis.opendocument.text"><office:automatic-styles><style:page-layout style:name="pm"><style:page-layout-properties fo:page-width="21cm" fo:page-height="29.7cm" fo:margin-left="2cm" fo:margin-right="2cm"/></style:page-layout><style:style style:name="P1" style:family="paragraph"><style:paragraph-properties fo:margin-right="12cm"/><style:text-properties fo:font-family="'Liberation Serif'" fo:font-size="12pt" fo:language="en" fo:country="US" {hyphenate}/></style:style></office:automatic-styles><office:master-styles><style:master-page style:name="Standard" style:page-layout-name="pm"/></office:master-styles><office:body><office:text><text:p text:style-name="P1">The internationalization of representational characteristics necessitates extraordinary hyphenation considerations nevertheless, and incomprehensibilities accumulate unquestionably.</text:p></office:text></office:body></office:document>"#
            );
            let app = App::new();
            app.open_bytes("h.fodt", bytes.as_bytes()).unwrap();
            let doc = typeset(&app, &setter(), &Options::default());
            let mut lines: Vec<(f32, String)> = Vec::new();
            for (_, y, text) in texts(&doc.pages[0]) {
                match lines.last_mut() {
                    Some((at, line)) if (*at - y).abs() < 0.01 => line.push_str(&text),
                    _ => lines.push((y, text)),
                }
            }
            lines
                .into_iter()
                .map(|(_, line)| line.trim().to_owned())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            lines(r#"fo:hyphenate="true""#),
            [
                "The internationalization of",
                "representational characteris-",
                "tics necessitates extraordi-",
                "nary hyphenation considera-",
                "tions nevertheless, and in-",
                "comprehensibilities accumu-",
                "late unquestionably.",
            ]
        );
        assert_eq!(lines("")[1], "representational");
    }

    /// A block whose style the document declares is set as that style says; one whose style it
    /// does not declare keeps the screen's face, in the document's default family.
    #[test]
    fn a_declared_style_sets_its_blocks_face() {
        let bytes = r#"<office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" office:mimetype="application/vnd.oasis.opendocument.text">
            <office:styles>
              <style:default-style style:family="paragraph"><style:text-properties fo:font-family="'Liberation Sans'" fo:font-size="12pt"/></style:default-style>
              <style:style style:name="Heading" style:family="paragraph"><style:text-properties fo:font-size="14pt"/></style:style>
              <style:style style:name="Heading_20_1" style:family="paragraph" style:parent-style-name="Heading"><style:text-properties fo:font-size="130%" fo:font-weight="bold" fo:font-family="'Liberation Mono'"/></style:style>
            </office:styles>
            <office:body><office:text>
              <text:h text:style-name="Heading_20_1" text:outline-level="1">declared</text:h>
              <text:h text:style-name="Undeclared" text:outline-level="1">undeclared</text:h>
              <text:p>plain</text:p>
            </office:text></office:body></office:document>"#;
        let app = App::new();
        app.open_bytes("s.fodt", bytes.as_bytes()).unwrap();
        let t = setter();
        let page = &typeset(&app, &t, &Options::default()).pages[0];
        let face = |want: &str| {
            page.ops.iter().find_map(|op| match op {
                Op::Text {
                    text, face, size, ..
                } if text == want => {
                    let f = t.fonts().face(*face);
                    Some((f.family.clone(), f.bold, *size))
                }
                _ => None,
            })
        };
        assert_eq!(
            face("declared"),
            Some(("Liberation Mono".into(), true, 18.2))
        );
        assert_eq!(
            face("undeclared"),
            Some(("Liberation Sans".into(), true, 21.6)),
            "the role's scale and weight"
        );
        assert_eq!(face("plain"), Some(("Liberation Sans".into(), false, 12.0)));
    }

    /// Two blocks with the same text and family are not one layout when anything else that sets
    /// their width differs: here the letter spacing and the case.
    #[test]
    fn a_layout_is_kept_for_its_whole_look() {
        let words = "word ".repeat(30);
        let bytes = format!(
            r#"<office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" office:mimetype="application/vnd.oasis.opendocument.text">
            <office:automatic-styles>
              <style:style style:name="S" style:family="text"><style:text-properties fo:letter-spacing="0.2cm"/></style:style>
              <style:style style:name="U" style:family="text"><style:text-properties fo:text-transform="uppercase"/></style:style>
              <style:style style:name="P" style:family="paragraph"><style:text-properties fo:letter-spacing="0.2cm"/></style:style>
            </office:automatic-styles>
            <office:body><office:text><text:p>{words}</text:p><text:p><text:span text:style-name="S">{words}</text:span></text:p><text:p><text:span text:style-name="U">{words}</text:span></text:p><text:p text:style-name="P">{words}</text:p></office:text></office:body></office:document>"#
        );
        let app = App::new();
        app.open_bytes("k.fodt", bytes.as_bytes()).unwrap();
        let set = typeset(&app, &setter(), &Options::default());
        let lines = |block: usize| {
            set.pages
                .iter()
                .flat_map(|page| &page.lines)
                .filter(|line| line.block == block)
                .count()
        };
        assert!(
            lines(1) > lines(0),
            "spaced {} plain {}",
            lines(1),
            lines(0)
        );
        assert!(lines(2) > lines(0), "upper {} plain {}", lines(2), lines(0));
        assert!(
            lines(3) > lines(0),
            "styled {} plain {}",
            lines(3),
            lines(0)
        );
    }

    /// `doc/odt-format.md` §5c facts 5 and 6, on paper: a declared style's space below and the
    /// next one's space above add, and the first paragraph's space above is applied.
    #[test]
    fn a_declared_styles_spacing_adds_as_writers_does() {
        let bytes = r#"<office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" office:mimetype="application/vnd.oasis.opendocument.text">
            <office:styles>
              <style:style style:name="A" style:family="paragraph"><style:paragraph-properties fo:margin-top="1cm" fo:margin-bottom="1cm"/></style:style>
              <style:style style:name="B" style:family="paragraph"><style:paragraph-properties fo:margin-top="0.5cm" fo:margin-bottom="0.5cm"/></style:style>
            </office:styles>
            <office:body><office:text><text:p text:style-name="A">one</text:p><text:p text:style-name="B">two</text:p></office:text></office:body></office:document>"#;
        let app = App::new();
        app.open_bytes("s.fodt", bytes.as_bytes()).unwrap();
        let page = &typeset(&app, &setter(), &Options::default()).pages[0];
        let y = |want: &str| texts(page).into_iter().find(|t| t.2 == want).unwrap().1;
        let cm = 72.0 / 2.54;
        let first_baseline = 56.6929 + cm + 10.6934;
        assert!(
            (y("one") - first_baseline as f32).abs() < 0.01,
            "{}",
            y("one")
        );
        let between = y("two") - y("one");
        assert!(
            (between - (13.7988 + 1.5 * cm) as f32).abs() < 0.01,
            "{between}"
        );
    }

    /// `doc/odt-format.md` §5c fact 8, on paper: a justified paragraph's lines reach the right
    /// margin except its last, and a centred line sits in the middle.
    #[test]
    fn justified_and_centred_paragraphs_are_drawn_so() {
        let words = "word ".repeat(60);
        let bytes = format!(
            r#"<office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" office:mimetype="application/vnd.oasis.opendocument.text">
            <office:automatic-styles>
              <style:style style:name="J" style:family="paragraph"><style:paragraph-properties fo:text-align="justify"/></style:style>
              <style:style style:name="C" style:family="paragraph"><style:paragraph-properties fo:text-align="center"/></style:style>
            </office:automatic-styles>
            <office:body><office:text><text:p text:style-name="J">{words}</text:p><text:p text:style-name="C">middle</text:p></office:text></office:body></office:document>"#
        );
        let app = App::new();
        app.open_bytes("a.fodt", bytes.as_bytes()).unwrap();
        let page = &typeset(&app, &setter(), &Options::default()).pages[0];
        // Every line's right end: the x of its last text op plus that op's advances.
        let mut ends: Vec<(f32, f32)> = Vec::new();
        for op in &page.ops {
            if let Op::Text {
                x, y, glyphs, text, ..
            } = op
            {
                let end = x + glyphs.iter().map(|g| g.x_advance).sum::<f32>();
                let visible = text.trim_end().len() == text.len();
                match ends.last_mut() {
                    Some((line, right)) if *line == *y => {
                        *right = right.max(if visible { end } else { *right });
                    }
                    _ => ends.push((*y, end)),
                }
            }
        }
        let right_margin = (595.2756 - 56.6929) as f32;
        let justified = &ends[..ends.len() - 2];
        assert!(justified.len() >= 2, "{ends:?}");
        for (_, right) in justified {
            // The last word's own trailing space is the one thing past the margin.
            assert!(
                (right - right_margin).abs() < 4.0,
                "{right} against {right_margin}"
            );
        }
        let last = ends[ends.len() - 2].1;
        assert!(
            last < right_margin - 100.0,
            "the paragraph's last line is not stretched"
        );
        let middle = texts(page).into_iter().find(|t| t.2 == "middle").unwrap();
        let centre = (56.6929 + 595.2756 - 56.6929) / 2.0;
        assert!(
            middle.0 > 250.0 && f64::from(middle.0) < centre,
            "{}",
            middle.0
        );
    }

    /// A declared style's side margins narrow the block: it starts its left margin in from the
    /// text area, and wraps that much and its right margin sooner.
    #[test]
    fn a_declared_styles_side_margins_narrow_the_block() {
        let words = "word ".repeat(60);
        let bytes = format!(
            r#"<office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" office:mimetype="application/vnd.oasis.opendocument.text">
            <office:styles><style:style style:name="Quote" style:family="paragraph"><style:paragraph-properties fo:margin-left="2cm" fo:margin-right="3cm"/></style:style></office:styles>
            <office:body><office:text><text:p text:style-name="Quote">{words}</text:p></office:text></office:body></office:document>"#
        );
        let app = App::new();
        app.open_bytes("q.fodt", bytes.as_bytes()).unwrap();
        let page = &typeset(&app, &setter(), &Options::default()).pages[0];
        let cm = 72.0 / 2.54;
        let mut right = 0.0_f32;
        for op in &page.ops {
            if let Op::Text {
                x, glyphs, text, ..
            } = op
            {
                assert!(
                    (x - (56.6929 + 2.0 * cm) as f32).abs() < 0.01 || *x > 100.0,
                    "{x}"
                );
                if !text.ends_with(' ') {
                    right = right.max(x + glyphs.iter().map(|g| g.x_advance).sum::<f32>());
                }
            }
        }
        let first = texts(page)[0].0;
        assert!(
            (first - (56.6929 + 2.0 * cm) as f32).abs() < 0.01,
            "{first}"
        );
        assert!(
            right <= (595.2756 - 56.6929 - 3.0 * cm) as f32 + 0.01,
            "{right}"
        );
    }

    /// `doc/odt-format.md` §5c fact 13, measured against LibreOffice 26.8: a tab goes to the
    /// paragraph's own stop, else the next whole `style:tab-stop-distance` — from its left
    /// margin, not the page's — and a right stop ends its text there. A header gets the same.
    #[test]
    fn tabs_go_to_the_stops_the_paragraph_style_sets() {
        let bytes = r#"<office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" office:mimetype="application/vnd.oasis.opendocument.text">
            <office:styles>
              <style:default-style style:family="paragraph"><style:paragraph-properties style:tab-stop-distance="1.25cm"/></style:default-style>
              <style:style style:name="Standard" style:family="paragraph"/>
              <style:style style:name="Stops" style:family="paragraph" style:parent-style-name="Standard"><style:paragraph-properties><style:tab-stops><style:tab-stop style:position="5cm"/><style:tab-stop style:position="17cm" style:type="right"/></style:tab-stops></style:paragraph-properties></style:style>
              <style:style style:name="Indented" style:family="paragraph" style:parent-style-name="Standard"><style:paragraph-properties fo:margin-left="2cm"/></style:style>
            </office:styles>
            <office:automatic-styles><style:page-layout style:name="pm1"><style:page-layout-properties fo:page-width="21cm" fo:page-height="29.7cm" fo:margin-top="2cm" fo:margin-bottom="2cm" fo:margin-left="2cm" fo:margin-right="2cm"/></style:page-layout></office:automatic-styles>
            <office:master-styles><style:master-page style:name="Standard" style:page-layout-name="pm1"><style:footer><text:p text:style-name="Stops">left<text:tab/><text:tab/>Page <text:page-number/></text:p></style:footer></style:master-page></office:master-styles>
            <office:body><office:text>
              <text:p text:style-name="Standard">Longer label<text:tab/>X</text:p>
              <text:p text:style-name="Stops">Name:<text:tab/>Value<text:tab/>Right</text:p>
              <text:p text:style-name="Indented">I<text:tab/>J</text:p>
            </office:text></office:body></office:document>"#;
        let app = App::new();
        app.open_bytes("t.fodt", bytes.as_bytes()).unwrap();
        let page = &typeset(&app, &setter(), &Options::default()).pages[0];
        let cm = (72.0 / 2.54) as f32;
        let margin = 2.0 * cm;
        let start = |want: &str| {
            texts(page)
                .into_iter()
                .find(|t| t.2 == want)
                .unwrap_or_else(|| panic!("{want} is drawn"))
                .0
        };
        let end = |want: &str| {
            page.ops
                .iter()
                .find_map(|op| match op {
                    Op::Text {
                        x, glyphs, text, ..
                    } if text == want => Some(x + glyphs.iter().map(|g| g.x_advance).sum::<f32>()),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("{want} is drawn"))
        };
        let near = |got: f32, want: f32| assert!((got - want).abs() < 0.05, "{got} vs {want}");
        near(start("X"), margin + 2.0 * 1.25 * cm);
        near(start("Value"), margin + 5.0 * cm);
        near(end("Right"), margin + 17.0 * cm);
        near(start("J"), 2.0 * margin + 1.25 * cm);
        near(end("Page 1"), margin + 17.0 * cm);
    }

    /// `doc/odt-format.md` §5c fact 16: a stop's leader fills its tab's gap — a table of
    /// contents' dots — and an index's link is drawn plain.
    #[test]
    fn a_leader_fills_the_gap_and_an_index_entry_is_not_a_navy_link() {
        let bytes = r##"<office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" xmlns:xlink="http://www.w3.org/1999/xlink" office:mimetype="application/vnd.oasis.opendocument.text">
            <office:styles><style:style style:name="Contents_20_1" style:family="paragraph"><style:paragraph-properties><style:tab-stops><style:tab-stop style:position="17cm" style:type="right" style:leader-style="dotted" style:leader-text="."/></style:tab-stops></style:paragraph-properties></style:style></office:styles>
            <office:body><office:text><text:table-of-content text:name="T"><text:index-body>
              <text:p text:style-name="Contents_20_1"><text:a xlink:href="#h" text:style-name="Index_20_Link">First<text:tab/>1</text:a></text:p>
            </text:index-body></text:table-of-content><text:h text:outline-level="1">First</text:h></office:text></office:body></office:document>"##;
        let app = App::new();
        app.open_bytes("toc.fodt", bytes.as_bytes()).unwrap();
        let page = &typeset(&app, &setter(), &Options::default()).pages[0];
        let dots = page.ops.iter().find_map(|op| match op {
            Op::Text { text, .. } if text.starts_with("...") => Some(text.len()),
            _ => None,
        });
        assert!(dots.is_some_and(|n| n > 50), "{dots:?}");
        let first = page.ops.iter().find_map(|op| match op {
            Op::Text { text, color, .. } if text == "First" => Some(*color),
            _ => None,
        });
        assert_eq!(first, Some(Rgb::BLACK));
        assert!(
            !page.ops.iter().any(|op| matches!(op, Op::Line { .. })),
            "no underline"
        );
    }

    /// `doc/odt-format.md` §5c fact 17: a first paragraph naming a master page of its own sets
    /// page 1 on it — no header here — and the pages after it on its `next-style-name`'s.
    #[test]
    fn the_first_page_wears_its_own_master_and_the_rest_the_next() {
        let fill: String = (0..70).fold(String::new(), |mut joined, i| {
            let _ = write!(joined, "<text:p>Line {i} of filler text.</text:p>");
            joined
        });
        let bytes = format!(
            r#"<office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" office:mimetype="application/vnd.oasis.opendocument.text">
            <office:automatic-styles>
              <style:style style:name="P1" style:family="paragraph" style:master-page-name="First_20_Page"/>
              <style:page-layout style:name="pm1"><style:page-layout-properties fo:page-width="21cm" fo:page-height="29.7cm" fo:margin-top="2cm" fo:margin-bottom="2cm" fo:margin-left="2cm" fo:margin-right="2cm"/><style:header-style><style:header-footer-properties fo:margin-bottom="0.5cm"/></style:header-style></style:page-layout>
            </office:automatic-styles>
            <office:master-styles>
              <style:master-page style:name="Standard" style:page-layout-name="pm1"><style:header><text:p>RUNNINGHEAD</text:p></style:header></style:master-page>
              <style:master-page style:name="First_20_Page" style:page-layout-name="pm1" style:next-style-name="Standard"/>
            </office:master-styles>
            <office:body><office:text><text:p text:style-name="P1">FIRSTLINE</text:p>{fill}</office:text></office:body></office:document>"#
        );
        let app = App::new();
        app.open_bytes("first.fodt", bytes.as_bytes()).unwrap();
        let typeset = typeset(&app, &setter(), &Options::default());
        assert!(typeset.pages.len() >= 2);
        let has = |page: &crate::ops::Page, want: &str| texts(page).iter().any(|t| t.2 == want);
        assert!(!has(&typeset.pages[0], "RUNNINGHEAD"));
        assert!(has(&typeset.pages[1], "RUNNINGHEAD"));
        let top = |page: &crate::ops::Page| texts(page)[0].1;
        assert!(
            top(&typeset.pages[0]) < top(&typeset.pages[1]),
            "page 1's text starts at the margin, page 2's under the header"
        );
    }

    /// A paragraph whose style names a master page opens a page on it **wherever it is**, not
    /// only first: a series of letters on one letterhead, each with its own page numbers
    /// (`style:page-number`), the master's own margins, and `Page 1 of 1` that is the letter's.
    #[test]
    fn every_paragraph_naming_a_master_opens_a_page_on_it() {
        let bytes = r#"<office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" office:mimetype="application/vnd.oasis.opendocument.text">
            <office:automatic-styles>
              <style:style style:name="Opening" style:family="paragraph" style:master-page-name="Letter"><style:paragraph-properties style:page-number="1"/></style:style>
              <style:page-layout style:name="pl1"><style:page-layout-properties fo:page-width="21cm" fo:page-height="29.7cm" fo:margin-top="1cm" fo:margin-bottom="2cm" fo:margin-left="2cm" fo:margin-right="2cm"/></style:page-layout>
              <style:page-layout style:name="pl2"><style:page-layout-properties fo:page-width="21cm" fo:page-height="29.7cm" fo:margin-top="3cm" fo:margin-bottom="2cm" fo:margin-left="2cm" fo:margin-right="2cm"/></style:page-layout>
            </office:automatic-styles>
            <office:master-styles>
              <style:master-page style:name="Standard" style:page-layout-name="pl2"/>
              <style:master-page style:name="Letter" style:page-layout-name="pl1" style:next-style-name="Standard"><style:footer><text:p>No. <text:page-number/> of <text:page-count/></text:p></style:footer></style:master-page>
            </office:master-styles>
            <office:body><office:text>
              <text:p text:style-name="Opening">FIRST</text:p><text:p>body one</text:p>
              <text:p text:style-name="Opening">SECOND</text:p><text:p>body two</text:p>
            </office:text></office:body></office:document>"#;
        let app = App::new();
        app.open_bytes("series.fodt", bytes.as_bytes()).unwrap();
        let doc = typeset(&app, &setter(), &Options::default());
        assert_eq!(doc.pages.len(), 2, "one page a letter");
        let on = |page: usize, want: &str| texts(&doc.pages[page]).iter().any(|t| t.2 == want);
        assert!(on(0, "FIRST") && on(1, "SECOND"));
        // Both pages are on the Letter master — its 1 cm top margin, not Standard's 3 cm — and
        // both are "No. 1 of 1": the second letter restarted the numbering.
        let first_top = |page: usize| texts(&doc.pages[page])[0].1;
        assert_eq!(first_top(0), first_top(1));
        assert!(first_top(0) < 40.0, "{}", first_top(0));
        let footer = |page: usize| {
            texts(&doc.pages[page])
                .into_iter()
                .map(|t| t.2)
                .collect::<Vec<_>>()
                .join(" ")
        };
        assert!(footer(0).contains("No. 1 of 1"), "{}", footer(0));
        assert!(footer(1).contains("No. 1 of 1"), "{}", footer(1));
    }

    /// A landscape section's text is set as wide as its own page, not at the portrait measure
    /// the document opened on: the same paragraph takes fewer lines there.
    #[test]
    fn a_master_with_a_wider_page_sets_its_text_wider() {
        let words = "word ".repeat(80);
        let bytes = format!(
            r#"<office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" office:mimetype="application/vnd.oasis.opendocument.text">
            <office:automatic-styles>
              <style:style style:name="Wide" style:family="paragraph" style:master-page-name="Landscape"/>
              <style:page-layout style:name="pl1"><style:page-layout-properties fo:page-width="21cm" fo:page-height="29.7cm" fo:margin-top="2cm" fo:margin-bottom="2cm" fo:margin-left="2cm" fo:margin-right="2cm"/></style:page-layout>
              <style:page-layout style:name="pl2"><style:page-layout-properties fo:page-width="29.7cm" fo:page-height="21cm" fo:margin-top="2cm" fo:margin-bottom="2cm" fo:margin-left="2cm" fo:margin-right="2cm"/></style:page-layout>
            </office:automatic-styles>
            <office:master-styles>
              <style:master-page style:name="Standard" style:page-layout-name="pl1"/>
              <style:master-page style:name="Landscape" style:page-layout-name="pl2"/>
            </office:master-styles>
            <office:body><office:text>
              <text:p>{words}</text:p><text:p text:style-name="Wide">{words}</text:p>
            </office:text></office:body></office:document>"#
        );
        let app = App::new();
        app.open_bytes("wide.fodt", bytes.as_bytes()).unwrap();
        let doc = typeset(&app, &setter(), &Options::default());
        assert_eq!(doc.pages.len(), 2);
        assert!(doc.pages[1].width > doc.pages[0].width);
        let lines = |page: usize| {
            let mut ys: Vec<i32> = texts(&doc.pages[page]).iter().map(|t| t.1 as i32).collect();
            ys.dedup();
            ys.len()
        };
        assert!(
            lines(1) < lines(0),
            "{} lines wide, {} narrow",
            lines(1),
            lines(0)
        );
    }

    /// A paragraph that opens a page of another width after a page break starts at that page's
    /// top, however much taller the text before it was at the other width.
    #[test]
    fn a_page_opened_after_a_break_on_a_wider_master_starts_at_its_top() {
        let words = "word ".repeat(400);
        let bytes = format!(
            r#"<office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" office:mimetype="application/vnd.oasis.opendocument.text">
            <office:automatic-styles>
              <style:style style:name="After" style:family="paragraph"><style:paragraph-properties fo:break-after="page"/></style:style>
              <style:style style:name="Wide" style:family="paragraph" style:master-page-name="Wide"/>
              <style:page-layout style:name="pl1"><style:page-layout-properties fo:page-width="21cm" fo:page-height="29.7cm" fo:margin-top="2cm" fo:margin-bottom="2cm" fo:margin-left="4cm" fo:margin-right="4cm"/></style:page-layout>
              <style:page-layout style:name="pl2"><style:page-layout-properties fo:page-width="21cm" fo:page-height="29.7cm" fo:margin-top="2cm" fo:margin-bottom="2cm" fo:margin-left="1cm" fo:margin-right="1cm"/></style:page-layout>
            </office:automatic-styles>
            <office:master-styles>
              <style:master-page style:name="Standard" style:page-layout-name="pl1"/>
              <style:master-page style:name="Wide" style:page-layout-name="pl2"/>
            </office:master-styles>
            <office:body><office:text>
              <text:p>{words}</text:p><text:p text:style-name="After">end</text:p><text:p text:style-name="Wide">top</text:p>
            </office:text></office:body></office:document>"#
        );
        let app = App::new();
        app.open_bytes("shift.fodt", bytes.as_bytes()).unwrap();
        let doc = typeset(&app, &setter(), &Options::default());
        let first = texts(&doc.pages[0])[0].1;
        let last = doc.pages.last().unwrap();
        let top = texts(last).iter().find(|t| t.2 == "top").unwrap().1;
        assert!((top - first).abs() < 0.01, "{top} against {first}");
    }

    /// `doc/odt-format.md` §5c facts 24 and 37: a master's first-page header is on the
    /// document's first page and its own header on the rest; with no first-page footer stated,
    /// the first page has none. A paragraph naming the master its page is on opens a page that
    /// is not a first one.
    #[test]
    fn a_runs_first_page_wears_the_masters_first_page_header() {
        let body = "<text:p>para</text:p>".repeat(70);
        let bytes = format!(
            r#"<office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" office:mimetype="application/vnd.oasis.opendocument.text">
            <office:automatic-styles>
              <style:style style:name="Again" style:family="paragraph" style:master-page-name="Standard"/>
              <style:page-layout style:name="pl"><style:page-layout-properties fo:page-width="21cm" fo:page-height="29.7cm" fo:margin-top="2cm" fo:margin-bottom="2cm" fo:margin-left="2cm" fo:margin-right="2cm"/></style:page-layout>
            </office:automatic-styles>
            <office:master-styles>
              <style:master-page style:name="Standard" style:page-layout-name="pl">
                <style:header><text:p>HEADREST</text:p></style:header>
                <style:header-first><text:p>HEADFIRST</text:p></style:header-first>
                <style:footer><text:p>FOOTREST</text:p></style:footer>
              </style:master-page>
            </office:master-styles>
            <office:body><office:text>{body}<text:p text:style-name="Again">again</text:p>{body}</office:text></office:body></office:document>"#
        );
        let app = App::new();
        app.open_bytes("first.fodt", bytes.as_bytes()).unwrap();
        let doc = typeset(&app, &setter(), &Options::default());
        let marginals: Vec<String> = doc
            .pages
            .iter()
            .map(|page| {
                let texts: Vec<String> = texts(page).into_iter().map(|t| t.2).collect();
                let has = |want: &str| texts.iter().any(|t| t == want);
                let again = if has("again") { "again " } else { "" };
                match (has("HEADFIRST"), has("HEADREST"), has("FOOTREST")) {
                    (true, false, false) => format!("{again}first"),
                    (false, true, true) => format!("{again}rest"),
                    other => format!("{again}{other:?}"),
                }
            })
            .collect();
        assert!(marginals.len() >= 4, "{marginals:?}");
        assert_eq!(marginals[0], "first");
        assert_eq!(marginals[1], "rest");
        let again = marginals
            .iter()
            .position(|m| m.starts_with("again"))
            .unwrap();
        assert_eq!(marginals[again], "again rest", "{marginals:?}");
        assert_eq!(marginals[again + 1], "rest", "{marginals:?}");
    }

    /// `doc/odt-format.md` §5c facts 35–37 on one document: a mirrored master swaps the margins
    /// of an even page and puts its left footer there; a master reached by another's
    /// `next-style-name` opens on its first-page footer even on a left page; a paragraph naming
    /// the master its page is on opens a plain page; and a right-only master skips a number.
    #[test]
    fn left_pages_mirror_and_a_right_only_master_skips_a_number() {
        let bytes = r#"<office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" office:mimetype="application/vnd.oasis.opendocument.text">
            <office:automatic-styles>
              <style:style style:name="Break" style:family="paragraph"><style:paragraph-properties fo:break-before="page"/></style:style>
              <style:style style:name="OnA" style:family="paragraph" style:master-page-name="A"/>
              <style:style style:name="OnB" style:family="paragraph" style:master-page-name="B"/>
              <style:style style:name="OnR" style:family="paragraph" style:master-page-name="R"/>
              <style:page-layout style:name="pm" style:page-usage="mirrored"><style:page-layout-properties fo:page-width="21cm" fo:page-height="29.7cm" fo:margin-top="2cm" fo:margin-bottom="2cm" fo:margin-left="4cm" fo:margin-right="1cm"/></style:page-layout>
              <style:page-layout style:name="pr" style:page-usage="right"><style:page-layout-properties fo:page-width="21cm" fo:page-height="29.7cm" fo:margin-top="2cm" fo:margin-bottom="2cm" fo:margin-left="4cm" fo:margin-right="1cm"/></style:page-layout>
            </office:automatic-styles>
            <office:master-styles>
              <style:master-page style:name="A" style:page-layout-name="pm" style:next-style-name="B"><style:footer><text:p>A</text:p></style:footer></style:master-page>
              <style:master-page style:name="B" style:page-layout-name="pm">
                <style:footer><text:p>B <text:page-number/></text:p></style:footer>
                <style:footer-left><text:p>left B <text:page-number/></text:p></style:footer-left>
                <style:footer-first><text:p>first B <text:page-number/></text:p></style:footer-first>
              </style:master-page>
              <style:master-page style:name="R" style:page-layout-name="pr" style:next-style-name="B"><style:footer><text:p>R <text:page-number/></text:p></style:footer></style:master-page>
            </office:master-styles>
            <office:body><office:text>
              <text:p text:style-name="OnA">one</text:p><text:p text:style-name="Break">two</text:p>
              <text:p text:style-name="Break">three</text:p><text:p text:style-name="OnB">four</text:p>
              <text:p text:style-name="OnR">five</text:p><text:p text:style-name="Break">six</text:p>
            </office:text></office:body></office:document>"#;
        let app = App::new();
        app.open_bytes("hands.fodt", bytes.as_bytes()).unwrap();
        let doc = typeset(&app, &setter(), &Options::default());
        let cm = 72.0 / 2.54;
        let pages: Vec<(f32, String)> = doc
            .pages
            .iter()
            .map(|page| {
                let texts = texts(page);
                let words: Vec<String> = texts.iter().map(|t| t.2.clone()).collect();
                (texts[0].0, words.join(" "))
            })
            .collect();
        let expected = [
            (4.0, "one A"),
            (1.0, "two first B 2"),
            (4.0, "three B 3"),
            (1.0, "four left B 4"),
            (4.0, "five R 5"),
            (1.0, "six first B 6"),
        ];
        assert_eq!(pages.len(), expected.len(), "{pages:?}");
        for ((x, words), (margin, want)) in pages.iter().zip(expected) {
            assert_eq!(words.split_whitespace().collect::<Vec<_>>().join(" "), want);
            assert!(
                (x - (margin * cm) as f32).abs() < 0.01,
                "{want}: {x} against {margin} cm"
            );
        }
    }

    /// `doc/odt-format.md` §5c fact 38: a footer paragraph's top border is drawn across the
    /// text area, above its line, and the footer still ends on the bottom margin — its text
    /// where it was without the border, and padding taking room inside it.
    #[test]
    fn a_footer_paragraphs_border_is_drawn_inside_the_footer() {
        let footer = |props: &str| {
            let bytes = format!(
                r#"<office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" office:mimetype="application/vnd.oasis.opendocument.text">
                <office:styles><style:style style:name="Ruled" style:family="paragraph"><style:paragraph-properties {props}/></style:style></office:styles>
                <office:automatic-styles><style:page-layout style:name="pm"><style:page-layout-properties fo:page-width="21cm" fo:page-height="29.7cm" fo:margin-top="2cm" fo:margin-bottom="2cm" fo:margin-left="4cm" fo:margin-right="1cm"/></style:page-layout></office:automatic-styles>
                <office:master-styles><style:master-page style:name="Standard" style:page-layout-name="pm"><style:footer><text:p text:style-name="Ruled">foot</text:p></style:footer></style:master-page></office:master-styles>
                <office:body><office:text><text:p>body</text:p></office:text></office:body></office:document>"#
            );
            let app = App::new();
            app.open_bytes("f.fodt", bytes.as_bytes()).unwrap();
            let page = typeset(&app, &setter(), &Options::default())
                .pages
                .remove(0);
            let foot = texts(&page).into_iter().find(|t| t.2 == "foot").unwrap();
            let rule = page.ops.iter().find_map(|op| match op {
                Op::Line {
                    from, to, width, ..
                } => Some((*from, *to, *width)),
                _ => None,
            });
            (foot.0, foot.1, rule)
        };
        let cm = (72.0 / 2.54) as f32;
        let (_, plain, none) = footer(r#"fo:margin-top="0cm""#);
        assert_eq!(none, None);
        let (x, y, rule) =
            footer(r#"fo:border-top="2pt solid #ff0000" fo:border-bottom="none" fo:padding="0cm""#);
        assert!((y - plain).abs() < 0.01, "{y} against {plain}");
        let (from, to, width) = rule.unwrap();
        assert!((from.0 - 4.0 * cm).abs() < 0.01 && (to.0 - 20.0 * cm).abs() < 0.01);
        assert!((width - 2.0).abs() < 0.01);
        assert!(
            from.1 < y,
            "the rule above the line: {} against {y}",
            from.1
        );
        let (padded_x, padded_y, _) = footer(
            r#"fo:border-top="2pt solid #ff0000" fo:border-bottom="none" fo:padding="0.3cm""#,
        );
        assert!((padded_y - (plain - 0.3 * cm)).abs() < 0.01, "{padded_y}");
        assert!((padded_x - (x + 0.3 * cm)).abs() < 0.01, "{padded_x}");
    }

    /// A line in a header — a fold mark — is drawn on every page set on its master, in the stroke
    /// its graphic style names and counted from the box that style names (§5c fact 49).
    #[test]
    fn a_headers_line_is_counted_from_the_box_its_style_names() {
        let drawn = |place: &str| {
            let bytes = format!(
                r##"<office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0" xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0" office:mimetype="application/vnd.oasis.opendocument.text">
            <office:automatic-styles>
              <style:style style:name="Mark" style:family="graphic"><style:graphic-properties svg:stroke-width="0.1cm" svg:stroke-color="#ff0000"{place}/></style:style>
              <style:page-layout style:name="pl"><style:page-layout-properties fo:page-width="21cm" fo:page-height="29.7cm" fo:margin-top="2cm" fo:margin-bottom="2cm" fo:margin-left="2cm" fo:margin-right="2cm"/></style:page-layout>
            </office:automatic-styles>
            <office:master-styles>
              <style:master-page style:name="Standard" style:page-layout-name="pl"><style:header><text:p>First</text:p><text:p>Head<draw:line draw:style-name="Mark" text:anchor-type="page" svg:x1="1cm" svg:y1="10.5cm" svg:x2="1.5cm" svg:y2="10.5cm"/></text:p></style:header></style:master-page>
            </office:master-styles>
            <office:body><office:text><text:p>body</text:p></office:text></office:body></office:document>"##
            );
            let app = App::new();
            app.open_bytes("marks.fodt", bytes.as_bytes()).unwrap();
            let doc = typeset(&app, &setter(), &Options::default());
            let lines: Vec<_> = doc.pages[0]
                .ops
                .iter()
                .filter_map(|op| match op {
                    crate::ops::Op::Line {
                        from, to, width, ..
                    } if (to.0 - from.0 - 14.17).abs() < 0.1 => Some((*from, *width)),
                    _ => None,
                })
                .collect();
            assert_eq!(lines.len(), 1, "{place}: {lines:?}");
            lines[0]
        };
        let cm = 72.0 / 2.54;
        let near = |got: f32, want: f32| (got - want).abs() < 0.1;

        // The paper's corner: 1 cm across, 10.5 cm down, 1 mm thick.
        let ((x, y), width) = drawn(r#" style:horizontal-rel="page" style:vertical-rel="page""#);
        assert!(near(x, cm) && near(y, 10.5 * cm), "{x} {y}");
        assert!((width - 2.83).abs() < 0.05);

        // Nothing stated is the anchoring paragraph: the text area's left edge, and the top of
        // the header's second paragraph, one 12 pt line below the 2 cm margin.
        let ((x, y), _) = drawn("");
        let line = y - 12.5 * cm;
        assert!(near(x, 3.0 * cm) && line > 10.0 && line < 18.0, "{x} {y}");
        assert_eq!(
            drawn(r#" style:horizontal-rel="paragraph" style:vertical-rel="paragraph""#).0,
            (x, y)
        );

        // The text area's left edge, and the body's top below the whole header.
        let ((content_x, content_y), _) =
            drawn(r#" style:horizontal-rel="page-content" style:vertical-rel="page-content""#);
        assert!(
            near(content_x, 3.0 * cm) && content_y > y + 10.0,
            "{content_y}"
        );
    }

    /// A picture that states no size — one imported from Markdown — is drawn at its own pixels,
    /// 96 to the inch, rather than left out.
    #[test]
    fn a_picture_without_a_size_is_drawn_at_its_own() {
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        png.extend_from_slice(&200u32.to_be_bytes());
        png.extend_from_slice(&100u32.to_be_bytes());
        assert_eq!(grind_text::picture::pixels(&png), Some((200, 100)));
        let gif = b"GIF89a\x40\x01\xf0\x00";
        assert_eq!(grind_text::picture::pixels(gif), Some((320, 240)));
        assert_eq!(grind_text::picture::pixels(b"not a picture"), None);
    }

    /// A style's `fo:break-before="page"` starts its paragraph on a page of its own.
    #[test]
    fn a_styles_page_break_starts_a_new_page() {
        let bytes = r#"<office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" office:mimetype="application/vnd.oasis.opendocument.text">
            <office:automatic-styles><style:style style:name="P1" style:family="paragraph"><style:paragraph-properties fo:break-before="page"/></style:style></office:automatic-styles>
            <office:body><office:text><text:p>one</text:p><text:p text:style-name="P1">two</text:p></office:text></office:body></office:document>"#;
        let app = App::new();
        app.open_bytes("b.fodt", bytes.as_bytes()).unwrap();
        let doc = typeset(&app, &setter(), &Options::default());
        let pages: Vec<Vec<String>> = doc
            .pages
            .iter()
            .map(|page| texts(page).into_iter().map(|t| t.2).collect())
            .collect();
        assert_eq!(pages, vec![vec!["one".to_owned()], vec!["two".to_owned()]]);
    }

    /// A declared style is the whole answer: what it does not state is the default, upright and
    /// regular at the default size — never the screen's guess for the block's role, which is
    /// what made Writer's `Subtitle` print in italic.
    #[test]
    fn a_declared_style_is_complete_without_the_screens_role() {
        let bytes = r#"<office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" office:mimetype="application/vnd.oasis.opendocument.text">
            <office:styles>
              <style:style style:name="Subtitle" style:family="paragraph"><style:text-properties fo:font-size="18pt"/></style:style>
              <style:style style:name="Heading_20_1" style:family="paragraph"/>
            </office:styles>
            <office:body><office:text>
              <text:p text:style-name="Subtitle">sub</text:p>
              <text:h text:style-name="Heading_20_1" text:outline-level="1">head</text:h>
            </office:text></office:body></office:document>"#;
        let app = App::new();
        app.open_bytes("c.fodt", bytes.as_bytes()).unwrap();
        let t = setter();
        let page = &typeset(&app, &t, &Options::default()).pages[0];
        let face = |want: &str| {
            page.ops.iter().find_map(|op| match op {
                Op::Text {
                    text, face, size, ..
                } if text == want => {
                    let f = t.fonts().face(*face);
                    Some((f.bold, f.italic, *size))
                }
                _ => None,
            })
        };
        assert_eq!(
            face("sub"),
            Some((false, false, 18.0)),
            "upright, as the style says nothing"
        );
        assert_eq!(
            face("head"),
            Some((false, false, 12.0)),
            "a heading style stating nothing is body text"
        );
    }

    /// A family nobody has falls back to the kind of face its declaration says it is.
    #[test]
    fn an_uninstalled_sans_family_prints_in_the_bundled_sans() {
        let bytes = r#"<office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0" office:mimetype="application/vnd.oasis.opendocument.text">
            <office:font-face-decls><style:font-face style:name="Adwaita Sans" svg:font-family="'Adwaita Sans'" style:font-family-generic="swiss"/></office:font-face-decls>
            <office:styles><style:style style:name="Standard" style:family="paragraph"><style:text-properties style:font-name="Adwaita Sans"/></style:style></office:styles>
            <office:body><office:text><text:p text:style-name="Standard">sans</text:p></office:text></office:body></office:document>"#;
        let app = App::new();
        app.open_bytes("g.fodt", bytes.as_bytes()).unwrap();
        let t = setter();
        let page = &typeset(&app, &t, &Options::default()).pages[0];
        let family = page.ops.iter().find_map(|op| match op {
            Op::Text { face, .. } => Some(t.fonts().face(*face).family.clone()),
            _ => None,
        });
        assert_eq!(family.as_deref(), Some("Liberation Sans"));
        assert_eq!(t.substitutions()[0].used, "Liberation Sans");
    }

    /// `doc/odt-format.md` §5c fact 9: the header sits on the top margin and the footer on the
    /// bottom one, the body between them less their spacing, and the page fields are each
    /// page's own — Writer's 48 lines a page, `pg 2 of 3`.
    /// §5c fact 29: pictures set as characters in a header paragraph with no text stand side by side
    /// on its foot, aligned as it is, and make it as tall as the tallest.
    #[test]
    fn a_headers_pictures_set_as_characters_stand_in_a_row() {
        let png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVR4nGP4z8AAAAMBAQDJ/pLvAAAAAElFTkSuQmCC";
        let frame = |w: &str, h: &str| {
            format!(
                r#"<draw:frame text:anchor-type="as-char" svg:width="{w}" svg:height="{h}"><draw:image><office:binary-data>{png}</office:binary-data></draw:image></draw:frame>"#
            )
        };
        let bytes = format!(
            r#"<office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0" xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" office:mimetype="application/vnd.oasis.opendocument.text">
            <office:automatic-styles>
              <style:style style:name="C" style:family="paragraph"><style:paragraph-properties fo:text-align="center"/></style:style>
              <style:page-layout style:name="pm1"><style:page-layout-properties fo:page-width="21cm" fo:page-height="29.7cm" fo:margin-top="1cm" fo:margin-bottom="2cm" fo:margin-left="2cm" fo:margin-right="2cm"/>
                <style:header-style><style:header-footer-properties fo:min-height="0cm" fo:margin-bottom="0.5cm"/></style:header-style></style:page-layout>
            </office:automatic-styles>
            <office:master-styles><style:master-page style:name="Standard" style:page-layout-name="pm1">
              <style:header><text:p text:style-name="C">{}{}</text:p></style:header>
            </style:master-page></office:master-styles>
            <office:body><office:text><text:p>body</text:p></office:text></office:body></office:document>"#,
            frame("4cm", "2cm"),
            frame("3cm", "1cm"),
        );
        let app = App::new();
        app.open_bytes("ac.fodt", bytes.as_bytes()).unwrap();
        let doc = typeset(&app, &setter(), &Options::default());
        let page = &doc.pages[0];
        let images: Vec<(f32, f32, f32)> = page
            .ops
            .iter()
            .filter_map(|op| match op {
                Op::Image { x, y, height, .. } => Some((*x, *y, *height)),
                _ => None,
            })
            .collect();
        let cm = (72.0 / 2.54) as f32;
        let near = |got: f32, want: f32| assert!((got - want).abs() < 0.01, "{got} vs {want}");
        assert_eq!(images.len(), 2);
        near(images[0].0, 7.0 * cm);
        near(images[1].0, 11.0 * cm);
        near(images[0].1 + images[0].2, 3.0 * cm);
        near(images[1].1 + images[1].2, 3.0 * cm);
        let body = texts(page)
            .into_iter()
            .find(|t| t.2 == "body")
            .expect("body");
        assert!(
            body.1 > 3.5 * cm,
            "below the header's 2 cm and its spacing: {}",
            body.1
        );
    }

    /// §5c facts 27 and 28: a header's paragraphs spaced as the body's, an empty one a line tall,
    /// and its picture placed by its graphic style without taking room.
    #[test]
    fn a_headers_spacing_and_picture_are_writers() {
        let png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVR4nGP4z8AAAAMBAQDJ/pLvAAAAAElFTkSuQmCC";
        let bytes = format!(
            r#"<office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0" xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" office:mimetype="application/vnd.oasis.opendocument.text">
            <office:automatic-styles>
              <style:style style:name="N" style:family="paragraph"><style:text-properties fo:font-family="'Liberation Sans'" fo:font-size="12pt"/></style:style>
              <style:style style:name="Sp" style:family="paragraph" style:parent-style-name="N"><style:paragraph-properties fo:margin-top="0.3cm" fo:margin-bottom="0.5cm" fo:margin-left="3cm"/></style:style>
              <style:style style:name="fr" style:family="graphic"><style:graphic-properties style:wrap="run-through" style:horizontal-pos="center" style:horizontal-rel="page" style:vertical-pos="from-top" style:vertical-rel="paragraph"/></style:style>
              <style:page-layout style:name="pm1"><style:page-layout-properties fo:page-width="21cm" fo:page-height="29.7cm" fo:margin-top="1cm" fo:margin-bottom="2cm" fo:margin-left="2cm" fo:margin-right="2cm"/>
                <style:header-style><style:header-footer-properties fo:min-height="0cm" fo:margin-bottom="0.5cm"/></style:header-style></style:page-layout>
            </office:automatic-styles>
            <office:master-styles><style:master-page style:name="Standard" style:page-layout-name="pm1">
              <style:header><text:p text:style-name="Sp">spaced</text:p><text:p text:style-name="N"><draw:frame draw:style-name="fr" text:anchor-type="paragraph" svg:y="1cm" svg:width="4cm" svg:height="2cm"><draw:image><office:binary-data>{png}</office:binary-data></draw:image></draw:frame></text:p><text:p text:style-name="N">after</text:p></style:header>
            </style:master-page></office:master-styles>
            <office:body><office:text><text:p text:style-name="N">body</text:p></office:text></office:body></office:document>"#
        );
        let app = App::new();
        app.open_bytes("hp.fodt", bytes.as_bytes()).unwrap();
        let doc = typeset(&app, &setter(), &Options::default());
        let page = &doc.pages[0];
        let texts = texts(page);
        let at = |want: &str| texts.iter().find(|t| t.2 == want).expect(want).clone();
        let cm = 72.0 / 2.54;
        let (spaced, after) = (at("spaced"), at("after"));
        assert!(
            (spaced.0 - 5.0 * cm).abs() < 0.01,
            "set in by its margin: {}",
            spaced.0
        );
        // Baseline to baseline: its line, its 0.5 cm below, and the empty paragraph's line.
        let line = (after.1 - spaced.1 - 0.5 * cm) / 2.0;
        assert!(
            (line - 13.8).abs() < 0.2,
            "the empty paragraph is a line tall: {line}"
        );
        let image = page
            .ops
            .iter()
            .find_map(|op| match op {
                Op::Image { x, y, width, .. } => Some((*x, *y, *width)),
                _ => None,
            })
            .expect("the picture is drawn");
        assert!(
            (image.0 - 8.5 * cm).abs() < 0.01,
            "centred on the paper: {}",
            image.0
        );
        assert!((image.2 - 4.0 * cm).abs() < 0.01);
        // 1 cm into its paragraph, which starts 0.3 cm, a line and 0.5 cm below the 1 cm margin.
        let top = 2.8 * cm + line;
        assert!((image.1 - top).abs() < 0.2, "{} vs {top}", image.1);
        let body = at("body");
        assert!(body.1 > after.1, "the picture takes no room");
        assert!(
            body.1 < image.1 + 2.0 * cm,
            "the picture takes no room: {}",
            body.1
        );
    }

    /// §5c fact 67: a page's footnotes stand at the foot of its body under their rule, their
    /// citations raised where they are cited, and endnotes get a page after the body.
    #[test]
    fn footnotes_stand_at_the_foot_and_endnotes_after_the_body() {
        let note = |id: &str, class: &str, citation: &str, text: &str| {
            format!(
                r#"<text:note text:id="{id}" text:note-class="{class}"><text:note-citation>{citation}</text:note-citation><text:note-body><text:p text:style-name="Footnote">{text}</text:p></text:note-body></text:note>"#
            )
        };
        let bytes = format!(
            r##"<office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" office:mimetype="application/vnd.oasis.opendocument.text">
            <office:styles>
              <style:style style:name="Footnote" style:family="paragraph"><style:paragraph-properties fo:margin-left="0.6cm" fo:text-indent="-0.6cm" fo:margin-top="0cm" fo:margin-bottom="0cm"/><style:text-properties fo:font-size="10pt"/></style:style>
            </office:styles>
            <office:automatic-styles>
              <style:page-layout style:name="pm1"><style:page-layout-properties fo:page-width="21cm" fo:page-height="29.7cm" fo:margin-top="2cm" fo:margin-bottom="2cm" fo:margin-left="2cm" fo:margin-right="2cm"><style:footnote-sep style:width="0.1cm" style:distance-before-sep="0.1cm" style:distance-after-sep="0.3cm" style:rel-width="50%" style:adjustment="center" style:color="#ff0000" style:line-style="solid"/></style:page-layout-properties></style:page-layout>
            </office:automatic-styles>
            <office:master-styles><style:master-page style:name="Standard" style:page-layout-name="pm1"/></office:master-styles>
            <office:body><office:text><text:p>cited{one} and{end} here</text:p></office:text></office:body></office:document>"##,
            one = note("n1", "footnote", "1", "Footnotetext"),
            end = note("e1", "endnote", "i", "Endnotetext"),
        );
        let app = App::new();
        app.open_bytes("notes.fodt", bytes.as_bytes()).unwrap();
        let doc = typeset(&app, &setter(), &Options::default());
        assert_eq!(doc.pages.len(), 2, "the body's page and the endnotes'");
        let cm = (72.0 / 2.54) as f32;
        let page = &doc.pages[0];
        let body = texts(page);
        let at = |word: &str| body.iter().find(|t| t.2.trim() == word).map(|t| (t.0, t.1));
        let (_, cited) = at("cited").expect("the body");
        let (raised_x, raised) = body
            .iter()
            .find(|t| t.2 == "1" && t.1 < 5.0 * cm)
            .map(|t| (t.0, t.1))
            .expect("the citation");
        assert!(raised < cited - 1.0, "raised: {raised} vs {cited}");
        assert!(raised_x > 2.5 * cm);
        // The note: its number at the margin, its text at the hanging indent, its line ending
        // at the foot of the body (27.7 cm).
        let (text_x, text_y) = at("Footnotetext").expect("the note");
        assert!((text_x - 2.6 * cm).abs() < 0.1 * cm, "{} cm", text_x / cm);
        assert!(
            text_y > 27.0 * cm && text_y < 27.7 * cm,
            "{} cm",
            text_y / cm
        );
        // The rule: red, half the text area, centred, 0.3 cm above the note's line.
        let rule = page
            .ops
            .iter()
            .find_map(|op| match op {
                Op::Rect {
                    x,
                    y,
                    width,
                    height,
                    color,
                } if *color == Rgb(0xff, 0, 0) => Some((*x, *y, *width, *height)),
                _ => None,
            })
            .expect("the rule");
        assert!((rule.0 - 6.25 * cm).abs() < 0.05 * cm && (rule.2 - 8.5 * cm).abs() < 0.05 * cm);
        assert!((rule.3 - 0.1 * cm).abs() < 0.01 * cm);
        assert!(!body.iter().any(|t| t.2.contains("Endnotetext")));
        let endnotes = texts(&doc.pages[1]);
        let (_, top) = endnotes
            .iter()
            .find(|t| t.2.trim() == "Endnotetext")
            .map(|t| (t.0, t.1))
            .expect("the endnote");
        assert!(top < 3.0 * cm, "from the top margin: {} cm", top / cm);
    }

    /// §5c fact 74: a tab among 11 pt runs in a 12 pt paragraph style does not make its line a
    /// 12 pt one.
    #[test]
    fn a_tab_is_measured_in_its_neighbours_font() {
        let pitch = |entry: &str| {
            let bytes = format!(
                r#"<office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" office:mimetype="application/vnd.oasis.opendocument.text">
                <office:styles><style:style style:name="Big" style:family="paragraph"><style:paragraph-properties fo:margin-top="0cm" fo:margin-bottom="0cm"/><style:text-properties fo:font-size="12pt"/></style:style></office:styles>
                <office:automatic-styles><style:style style:name="Small" style:family="text"><style:text-properties fo:font-size="8pt"/></style:style></office:automatic-styles>
                <office:body><office:text><text:p text:style-name="Big">{entry}</text:p><text:p text:style-name="Big">{entry}</text:p></office:text></office:body></office:document>"#
            );
            let app = App::new();
            app.open_bytes("tab.fodt", bytes.as_bytes()).unwrap();
            let texts = texts(&typeset(&app, &setter(), &Options::default()).pages[0]);
            let ys: Vec<f32> = texts
                .iter()
                .filter(|t| t.2 == "Entry")
                .map(|t| t.1)
                .collect();
            ys[1] - ys[0]
        };
        let plain = pitch(r#"<text:span text:style-name="Small">Entry</text:span>"#);
        let tabbed = pitch(
            r#"<text:span text:style-name="Small">Entry</text:span><text:span text:style-name="Small"><text:tab/>4</text:span>"#,
        );
        assert!((tabbed - plain).abs() < 0.01, "{tabbed} vs {plain}");
    }

    /// §5c fact 73: in a table whose cells are styled, one with no style has no padding — its
    /// text starts at its column's edge.
    #[test]
    fn an_unstyled_cell_has_no_padding() {
        let bytes = r#"<office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" office:mimetype="application/vnd.oasis.opendocument.text">
            <office:automatic-styles>
              <style:style style:name="C" style:family="table-cell"><style:table-cell-properties fo:padding="0.5cm"/></style:style>
              <style:style style:name="T" style:family="table"><style:table-properties style:width="17cm" table:align="left"/></style:style>
              <style:style style:name="col" style:family="table-column"><style:table-column-properties style:column-width="8.5cm"/></style:style>
              <style:page-layout style:name="pm1"><style:page-layout-properties fo:page-width="21cm" fo:page-height="29.7cm" fo:margin-top="2cm" fo:margin-bottom="2cm" fo:margin-left="2cm" fo:margin-right="2cm"/></style:page-layout>
            </office:automatic-styles>
            <office:master-styles><style:master-page style:name="Standard" style:page-layout-name="pm1"/></office:master-styles>
            <office:body><office:text><table:table table:style-name="T"><table:table-column table:style-name="col" table:number-columns-repeated="2"/><table:table-row><table:table-cell table:style-name="C"><text:p>styled</text:p></table:table-cell><table:table-cell><text:p>bare</text:p></table:table-cell></table:table-row></table:table></office:text></office:body></office:document>"#;
        let app = App::new();
        app.open_bytes("bare.fodt", bytes.as_bytes()).unwrap();
        let texts = texts(&typeset(&app, &setter(), &Options::default()).pages[0]);
        let at = |w: &str| texts.iter().find(|t| t.2 == w).map(|t| (t.0, t.1)).unwrap();
        let cm = (72.0 / 2.54) as f32;
        assert!((at("styled").0 - 2.5 * cm).abs() < 0.05 * cm, "padded");
        assert!(
            (at("bare").0 - 10.5 * cm).abs() < 0.05 * cm,
            "{} cm",
            at("bare").0 / cm
        );
        assert!(at("bare").1 < at("styled").1, "no padding above it either");
    }

    /// §5c fact 71: a cell spanning three rows of one line each, itself ten lines, leaves the
    /// rows their own heights and makes the last one taller.
    #[test]
    fn a_cell_spanning_rows_grows_the_last_of_them() {
        let lines = (0..10).fold(String::new(), |mut out, i| {
            let _ = write!(out, "<text:p>L{i}</text:p>");
            out
        });
        let row = |at: usize, first: &str| {
            format!(
                r#"<table:table-row>{first}<table:table-cell office:value-type="string"><text:p>R{at}</text:p></table:table-cell></table:table-row>"#
            )
        };
        let bytes = format!(
            r#"<office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0" office:mimetype="application/vnd.oasis.opendocument.text">
            <office:body><office:text><table:table><table:table-column table:number-columns-repeated="2"/>{}{}{}</table:table><text:p>after</text:p></office:text></office:body></office:document>"#,
            row(
                0,
                &format!(
                    r#"<table:table-cell table:number-rows-spanned="3">{lines}</table:table-cell>"#
                )
            ),
            row(1, "<table:covered-table-cell/>"),
            row(2, "<table:covered-table-cell/>"),
        );
        let app = App::new();
        app.open_bytes("span.fodt", bytes.as_bytes()).unwrap();
        let texts = texts(&typeset(&app, &setter(), &Options::default()).pages[0]);
        let y = |w: &str| texts.iter().find(|t| t.2 == w).unwrap().1;
        let line = y("L1") - y("L0");
        assert!(
            (y("R1") - y("R0") - (y("R2") - y("R1"))).abs() < 0.01,
            "even rows"
        );
        assert!(y("R2") - y("R0") < 3.0 * line, "{texts:?}");
        assert!(y("after") > y("L9"));
    }

    /// §5c fact 69: with `ContinuousEndnotes`, the endnotes follow the body's last line on its
    /// page, under a two-inch rule, rather than a page of their own.
    #[test]
    fn continuous_endnotes_follow_the_text() {
        let bytes = r#"<office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" xmlns:config="urn:oasis:names:tc:opendocument:xmlns:config:1.0" office:mimetype="application/vnd.oasis.opendocument.text">
            <office:settings><config:config-item-set config:name="ooo:configuration-settings"><config:config-item config:name="ContinuousEndnotes" config:type="boolean">true</config:config-item></config:config-item-set></office:settings>
            <office:body><office:text><text:p>cited<text:note text:id="e1" text:note-class="endnote"><text:note-citation>i</text:note-citation><text:note-body><text:p>Endnotetext</text:p></text:note-body></text:note> here</text:p></office:text></office:body></office:document>"#;
        let app = App::new();
        app.open_bytes("end.fodt", bytes.as_bytes()).unwrap();
        let doc = typeset(&app, &setter(), &Options::default());
        assert_eq!(doc.pages.len(), 1, "on the body's page");
        let page = &doc.pages[0];
        let body = texts(page);
        let y = |word: &str| body.iter().find(|t| t.2.trim() == word).map(|t| t.1);
        let note = body
            .iter()
            .find(|t| t.2.contains("Endnotetext"))
            .map(|t| t.1)
            .expect("the endnote");
        let cited = y("cited").expect("the body");
        let cm = (72.0 / 2.54) as f32;
        let rule = page
            .ops
            .iter()
            .find_map(|op| match op {
                Op::Rect { y, width, .. } => Some((*y, *width)),
                _ => None,
            })
            .expect("the rule");
        assert!(
            (rule.1 - 5.08 * cm).abs() < 0.01,
            "two inches: {}",
            rule.1 / cm
        );
        assert!(cited < rule.0 && rule.0 < note, "{cited} {rule:?} {note}");
    }

    /// §5c fact 66: a picture set as a character among words is drawn in its line, its foot on
    /// the baseline or centred on the text's height as its style says, and only its line grows.
    #[test]
    fn a_picture_among_words_is_drawn_in_its_line() {
        let png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVR4nGP4z8AAAAMBAQDJ/pLvAAAAAElFTkSuQmCC";
        let paragraph = |style: &str| {
            format!(
                r#"<text:p>aaa <draw:frame draw:style-name="{style}" text:anchor-type="as-char" svg:width="2cm" svg:height="1cm"><draw:image><office:binary-data>{png}</office:binary-data></draw:image></draw:frame> bbb</text:p><text:p>after</text:p>"#
            )
        };
        let bytes = format!(
            r#"<office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0" xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" office:mimetype="application/vnd.oasis.opendocument.text">
            <office:automatic-styles>
              <style:style style:name="foot" style:family="graphic"/>
              <style:style style:name="mid" style:family="graphic"><style:graphic-properties style:vertical-pos="middle" style:vertical-rel="text"/></style:style>
            </office:automatic-styles>
            <office:body><office:text>{foot}{mid}</office:text></office:body></office:document>"#,
            foot = paragraph("foot"),
            mid = paragraph("mid"),
        );
        let app = App::new();
        app.open_bytes("inline.fodt", bytes.as_bytes()).unwrap();
        let doc = typeset(&app, &setter(), &Options::default());
        let page = &doc.pages[0];
        let cm = (72.0 / 2.54) as f32;
        let pictures: Vec<(f32, f32, f32)> = page
            .ops
            .iter()
            .filter_map(|op| match op {
                Op::Image { x, y, height, .. } => Some((*x, *y, *height)),
                _ => None,
            })
            .collect();
        assert_eq!(pictures.len(), 2, "{pictures:?}");
        let texts = texts(page);
        let at = |word: &str| {
            texts
                .iter()
                .filter(|t| t.2.trim() == word)
                .map(|t| t.1)
                .collect::<Vec<_>>()
        };
        let (aaa, bbb, after) = (at("aaa"), at("bbb"), at("after"));
        assert!(
            !texts.iter().any(|t| t.2.contains('\u{fffc}')),
            "no placeholder drawn"
        );
        // Its foot on the words' baseline, and the words after it on the same one.
        assert!((pictures[0].1 + pictures[0].2 - aaa[0]).abs() < 0.5);
        assert!((bbb[0] - aaa[0]).abs() < 0.01);
        assert!(pictures[0].0 > 1.0 * cm && pictures[0].0 < 3.0 * cm);
        // Only its own line grew: the next paragraph one ordinary line below its baseline.
        assert!(
            after[0] - aaa[0] < 0.8 * cm,
            "{} cm",
            (after[0] - aaa[0]) / cm
        );
        assert!(
            aaa[0] - pictures[0].1 > 0.95 * cm,
            "the line grew to hold it"
        );
        // Centred on the text: its middle level with the middle of the words' height.
        let middle = pictures[1].1 + pictures[1].2 / 2.0;
        assert!(
            middle < aaa[1] && middle > aaa[1] - 0.4 * cm,
            "{middle} vs {}",
            aaa[1]
        );
    }

    /// §5c fact 65: in a table cell, a picture the text runs through measures across from the
    /// cell's box inside its padding, whatever its paragraph's indent, and takes no room.
    #[test]
    fn a_picture_the_text_runs_through_a_cell_measures_from_the_cell() {
        let png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVR4nGP4z8AAAAMBAQDJ/pLvAAAAAElFTkSuQmCC";
        let bytes = format!(
            r#"<office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0" xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0" xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" office:mimetype="application/vnd.oasis.opendocument.text">
            <office:automatic-styles>
              <style:style style:name="M" style:family="paragraph"><style:paragraph-properties fo:margin-left="1cm" fo:margin-top="0cm" fo:margin-bottom="0cm"/></style:style>
              <style:style style:name="C" style:family="table-cell"><style:table-cell-properties fo:padding="0.5cm"/></style:style>
              <style:style style:name="fr" style:family="graphic"><style:graphic-properties style:wrap="run-through" style:horizontal-pos="from-left" style:horizontal-rel="paragraph" style:vertical-pos="from-top" style:vertical-rel="paragraph"/></style:style>
              <style:page-layout style:name="pm1"><style:page-layout-properties fo:page-width="21cm" fo:page-height="29.7cm" fo:margin-top="2cm" fo:margin-bottom="2cm" fo:margin-left="2cm" fo:margin-right="2cm"/></style:page-layout>
            </office:automatic-styles>
            <office:master-styles><style:master-page style:name="Standard" style:page-layout-name="pm1"/></office:master-styles>
            <office:body><office:text>
              <table:table><table:table-column table:number-columns-repeated="2"/><table:table-row>
                <table:table-cell table:style-name="C"><text:p text:style-name="M"><draw:frame draw:style-name="fr" text:anchor-type="paragraph" svg:x="1cm" svg:y="0.5cm" svg:width="2cm" svg:height="1cm"><draw:image><office:binary-data>{png}</office:binary-data></draw:image></draw:frame>words</text:p></table:table-cell>
                <table:table-cell table:style-name="C"><text:p>right</text:p></table:table-cell>
              </table:table-row></table:table>
            </office:text></office:body></office:document>"#
        );
        let app = App::new();
        app.open_bytes("cell.fodt", bytes.as_bytes()).unwrap();
        let doc = typeset(&app, &setter(), &Options::default());
        let page = &doc.pages[0];
        let cm = 72.0 / 2.54;
        let (x, y) = page
            .ops
            .iter()
            .find_map(|op| match op {
                Op::Image { x, y, .. } => Some((f64::from(*x), f64::from(*y))),
                _ => None,
            })
            .expect("the picture");
        assert!((x - 3.5 * cm).abs() < 0.5, "{} cm", x / cm);
        assert!((y - 3.0 * cm).abs() < 0.5, "{} cm", y / cm);
        let texts = texts(page);
        let words = texts.iter().find(|t| t.2 == "words").expect("its words");
        assert!(
            (f64::from(words.0) - 3.5 * cm).abs() < 0.5,
            "beside it, not after it"
        );
    }

    /// §5c fact 64: a frame's cut reaches the page as fractions of the picture's own size — a
    /// 1 px PNG at 96 to the inch, 0.2646 mm square — for a figure and for a floating picture.
    #[test]
    fn a_cropped_picture_is_cut_on_paper_by_its_own_size() {
        let png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVR4nGP4z8AAAAMBAQDJ/pLvAAAAAElFTkSuQmCC";
        let frame = |anchor: &str| {
            format!(
                r#"<draw:frame draw:style-name="fr" text:anchor-type="{anchor}" svg:width="4cm" svg:height="2cm"><draw:image><office:binary-data>{png}</office:binary-data></draw:image></draw:frame>"#
            )
        };
        let bytes = format!(
            r#"<office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0" xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" office:mimetype="application/vnd.oasis.opendocument.text">
            <office:automatic-styles>
              <style:style style:name="fr" style:family="graphic"><style:graphic-properties style:wrap="run-through" style:horizontal-pos="from-left" style:horizontal-rel="paragraph" style:vertical-pos="from-top" style:vertical-rel="paragraph" fo:clip="rect(0mm, 0.1mm, 0.05mm, 0mm)"/></style:style>
            </office:automatic-styles>
            <office:body><office:text>
              <text:p>{figure}</text:p>
              <text:p>{floating}words</text:p>
            </office:text></office:body></office:document>"#,
            figure = frame("as-char"),
            floating = frame("paragraph"),
        );
        let app = App::new();
        app.open_bytes("crop.fodt", bytes.as_bytes()).unwrap();
        let doc = typeset(&app, &setter(), &Options::default());
        let crops: Vec<[f32; 4]> = doc.pages[0]
            .ops
            .iter()
            .filter_map(|op| match op {
                Op::Image { crop, .. } => Some(*crop),
                _ => None,
            })
            .collect();
        assert_eq!(crops.len(), 2, "{crops:?}");
        let own = 25.4 / 96.0;
        for crop in crops {
            let expected = [0.0, 0.1 / own, 0.05 / own, 0.0];
            for (got, want) in crop.iter().zip(expected) {
                assert!((f64::from(*got) - want).abs() < 0.001, "{crop:?}");
            }
        }
    }

    /// §5c fact 63: a picture the body's text runs through takes no room, across from the text
    /// area whatever the paragraph's margin, down from where its space above begins.
    #[test]
    fn a_picture_the_text_runs_through_takes_no_room() {
        let png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVR4nGP4z8AAAAMBAQDJ/pLvAAAAAElFTkSuQmCC";
        let frame = |y: &str| {
            format!(
                r#"<draw:frame draw:style-name="fr" text:anchor-type="paragraph" svg:x="1cm" svg:y="{y}" svg:width="4cm" svg:height="2cm"><draw:image><office:binary-data>{png}</office:binary-data></draw:image></draw:frame>"#
            )
        };
        let bytes = format!(
            r#"<office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0" xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" office:mimetype="application/vnd.oasis.opendocument.text">
            <office:automatic-styles>
              <style:style style:name="N" style:family="paragraph"><style:paragraph-properties fo:margin-top="0cm" fo:margin-bottom="0cm"/><style:text-properties fo:font-family="'Liberation Sans'" fo:font-size="12pt"/></style:style>
              <style:style style:name="M" style:family="paragraph" style:parent-style-name="N"><style:paragraph-properties fo:margin-left="3cm" fo:margin-top="1cm" fo:margin-bottom="0cm"/></style:style>
              <style:style style:name="fr" style:family="graphic"><style:graphic-properties style:wrap="run-through" style:horizontal-pos="from-left" style:horizontal-rel="paragraph" style:vertical-pos="from-top" style:vertical-rel="paragraph"/></style:style>
              <style:page-layout style:name="pm1"><style:page-layout-properties fo:page-width="21cm" fo:page-height="29.7cm" fo:margin-top="2cm" fo:margin-bottom="2cm" fo:margin-left="2cm" fo:margin-right="2cm"/></style:page-layout>
            </office:automatic-styles>
            <office:master-styles><style:master-page style:name="Standard" style:page-layout-name="pm1"/></office:master-styles>
            <office:body><office:text>
              <text:p text:style-name="N">before</text:p>
              <text:p text:style-name="N">{alone}</text:p>
              <text:p text:style-name="N">middle</text:p>
              <text:p text:style-name="M">{beside}words</text:p>
              <text:p text:style-name="N">after</text:p>
            </office:text></office:body></office:document>"#,
            alone = frame("-0.5cm"),
            beside = frame("0.5cm"),
        );
        let app = App::new();
        app.open_bytes("float.fodt", bytes.as_bytes()).unwrap();
        let doc = typeset(&app, &setter(), &Options::default());
        let page = &doc.pages[0];
        let texts = texts(page);
        let at = |want: &str| texts.iter().find(|t| t.2 == want).expect(want).clone();
        let cm = 72.0 / 2.54;
        let (before, middle, words, after) = (at("before"), at("middle"), at("words"), at("after"));
        // Baseline to baseline over the paragraph holding only the picture: two lines.
        let line = (middle.1 - before.1) / 2.0;
        assert!(
            (line - (after.1 - words.1)).abs() < 0.01,
            "a paragraph holding only the picture is one line, like any: {line}"
        );
        assert!(
            (words.0 - 5.0 * cm).abs() < 0.01,
            "words keep their place: {}",
            words.0
        );
        let images: Vec<(f32, f32, f32)> = page
            .ops
            .iter()
            .filter_map(|op| match op {
                Op::Image { x, y, width, .. } => Some((*x, *y, *width)),
                _ => None,
            })
            .collect();
        assert_eq!(images.len(), 2, "both drawn: {images:?}");
        for (x, _, width) in &images {
            assert!((x - 3.0 * cm).abs() < 0.01, "1 cm into the text area: {x}");
            assert!((width - 4.0 * cm).abs() < 0.01);
        }
        // The second paragraph's top is the first's line below the 2 cm margin; the fourth's is
        // where its 1 cm of space above begins, a line below `middle`'s top.
        let first_top = 2.0 * cm;
        assert!(
            (images[0].1 - (first_top + line - 0.5 * cm)).abs() < 0.2,
            "raised above its paragraph: {}",
            images[0].1
        );
        assert!(
            (images[1].1 - (first_top + 3.0 * line + 0.5 * cm)).abs() < 0.2,
            "from before its space above: {}",
            images[1].1
        );
    }

    #[test]
    fn headers_and_footers_take_their_room_and_number_the_pages() {
        let lines: String = (1..=120).fold(String::new(), |mut joined, i| {
            let _ = write!(joined, r#"<text:p text:style-name="N">fill {i}</text:p>"#);
            joined
        });
        let bytes = format!(
            r#"<office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" office:mimetype="application/vnd.oasis.opendocument.text">
            <office:automatic-styles>
              <style:style style:name="N" style:family="paragraph"><style:text-properties fo:font-family="'Liberation Serif'" fo:font-size="12pt"/></style:style>
              <style:style style:name="H" style:family="paragraph"><style:paragraph-properties fo:text-align="center"/><style:text-properties fo:font-family="'Liberation Serif'" fo:font-size="12pt"/></style:style>
              <style:page-layout style:name="pm1"><style:page-layout-properties fo:page-width="21cm" fo:page-height="29.7cm" fo:margin-top="2cm" fo:margin-bottom="2cm" fo:margin-left="2cm" fo:margin-right="2cm"/>
                <style:header-style><style:header-footer-properties fo:min-height="0cm" fo:margin-bottom="0.5cm"/></style:header-style>
                <style:footer-style><style:header-footer-properties fo:min-height="0cm" fo:margin-top="0.5cm"/></style:footer-style></style:page-layout>
            </office:automatic-styles>
            <office:master-styles><style:master-page style:name="Standard" style:page-layout-name="pm1">
              <style:header><text:p text:style-name="N">headline</text:p></style:header>
              <style:footer><text:p text:style-name="H">pg <text:page-number>1</text:page-number> of <text:page-count>3</text:page-count></text:p></style:footer>
            </style:master-page></office:master-styles>
            <office:body><office:text>{lines}</office:text></office:body></office:document>"#
        );
        let app = App::new();
        app.open_bytes("hf.fodt", bytes.as_bytes()).unwrap();
        let doc = typeset(&app, &setter(), &Options::default());
        let body: Vec<usize> = doc
            .pages
            .iter()
            .map(|page| {
                texts(page)
                    .iter()
                    .filter(|t| t.2.starts_with("fill"))
                    .count()
            })
            .collect();
        assert_eq!(body, vec![48, 48, 24]);
        let page = &doc.pages[1];
        let texts = texts(page);
        let header = texts
            .iter()
            .find(|t| t.2 == "headline")
            .expect("a header on page 2");
        assert!(
            (header.1 - (56.6929 + 10.6934) as f32).abs() < 0.01,
            "{}",
            header.1
        );
        let footer = texts
            .iter()
            .find(|t| t.2 == "pg 2 of 3")
            .expect("page 2's own footer");
        let bottom_baseline = 841.8898 - 56.6929 - 13.7988 + 10.6934;
        assert!(
            (footer.1 - bottom_baseline as f32).abs() < 0.01,
            "{}",
            footer.1
        );
        assert!(footer.0 > 250.0, "centred: {}", footer.0);
        let first = texts.iter().find(|t| t.2.starts_with("fill")).unwrap();
        let below_header = 56.6929 + 13.7988 + 72.0 / 2.54 * 0.5 + 10.6934;
        assert!((first.1 - below_header as f32).abs() < 0.01, "{}", first.1);
    }

    /// fact 10 on paper: a double-spaced paragraph's lines are 27.6 pt apart and its first line
    /// sits where a single-spaced one would.
    #[test]
    fn a_double_spaced_paragraph_doubles_its_lines() {
        let words = "word ".repeat(50);
        let bytes = format!(
            r#"<office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" office:mimetype="application/vnd.oasis.opendocument.text">
            <office:automatic-styles><style:style style:name="D" style:family="paragraph"><style:paragraph-properties fo:line-height="200%"/></style:style></office:automatic-styles>
            <office:body><office:text><text:p text:style-name="D">{words}</text:p></office:text></office:body></office:document>"#
        );
        let app = App::new();
        app.open_bytes("d.fodt", bytes.as_bytes()).unwrap();
        let page = &typeset(&app, &setter(), &Options::default()).pages[0];
        let mut ys: Vec<f32> = texts(page).iter().map(|t| t.1).collect();
        ys.dedup();
        assert!(
            (ys[0] - (56.6929 + 10.6934) as f32).abs() < 0.01,
            "{}",
            ys[0]
        );
        assert!((ys[1] - ys[0] - 27.5977).abs() < 0.01, "{}", ys[1] - ys[0]);
    }

    /// fact 11 on paper: a 2 cm first-line indent starts the first line 2 cm in and the rest at
    /// the margin.
    #[test]
    fn a_first_line_indent_moves_the_first_line_alone() {
        let words = "word ".repeat(50);
        let bytes = format!(
            r#"<office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" office:mimetype="application/vnd.oasis.opendocument.text">
            <office:automatic-styles><style:style style:name="I" style:family="paragraph"><style:paragraph-properties fo:text-indent="2cm"/></style:style></office:automatic-styles>
            <office:body><office:text><text:p text:style-name="I">{words}</text:p></office:text></office:body></office:document>"#
        );
        let app = App::new();
        app.open_bytes("i.fodt", bytes.as_bytes()).unwrap();
        let page = &typeset(&app, &setter(), &Options::default()).pages[0];
        let texts = texts(page);
        let first_line = texts[0].1;
        let first_x = texts
            .iter()
            .filter(|t| t.1 == first_line)
            .map(|t| t.0)
            .fold(f32::MAX, f32::min);
        let second_x = texts
            .iter()
            .filter(|t| t.1 > first_line)
            .map(|t| t.0)
            .fold(f32::MAX, f32::min);
        assert!(
            (first_x - (56.6929 + 72.0 / 2.54 * 2.0) as f32).abs() < 0.01,
            "{first_x}"
        );
        assert!((second_x - 56.6929).abs() < 0.01, "{second_x}");
    }

    /// `doc/odt-format.md` §5c fact 47: a header's `fo:min-height` counts its spacing — 2 cm
    /// with 0.5 cm below starts the body 2 cm under the top margin, not 2.5 cm.
    #[test]
    fn a_headers_min_height_counts_its_spacing() {
        let bytes = r#"<office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" office:mimetype="application/vnd.oasis.opendocument.text">
            <office:automatic-styles><style:page-layout style:name="pm"><style:page-layout-properties fo:page-width="21cm" fo:page-height="29.7cm" fo:margin-top="2cm" fo:margin-bottom="2cm" fo:margin-left="2cm" fo:margin-right="2cm"/><style:header-style><style:header-footer-properties fo:min-height="2cm" fo:margin-bottom="0.5cm"/></style:header-style><style:footer-style><style:header-footer-properties fo:min-height="2cm" fo:margin-top="0.5cm"/></style:footer-style></style:page-layout></office:automatic-styles>
            <office:master-styles><style:master-page style:name="Standard" style:page-layout-name="pm"><style:header><text:p>HEAD</text:p></style:header><style:footer><text:p>FOOT</text:p></style:footer></style:master-page></office:master-styles>
            <office:body><office:text><text:p>body</text:p></office:text></office:body></office:document>"#;
        let app = App::new();
        app.open_bytes("h.fodt", bytes.as_bytes()).unwrap();
        let page = &typeset(&app, &setter(), &Options::default()).pages[0];
        let texts = texts(page);
        let y = |w: &str| texts.iter().find(|t| t.2 == w).unwrap().1;
        let cm = (72.0 / 2.54) as f32;
        assert!((y("body") - y("HEAD") - 2.0 * cm).abs() < 0.01, "{texts:?}");
        assert!(((29.7 - 2.0 - 1.5) * cm - (y("FOOT") - (y("HEAD") - 2.0 * cm))).abs() < 0.01);
    }

    /// `doc/odt-format.md` §5c fact 68: a header's lines at 150% each get the extra below them
    /// but the last, so the body starts one plain line under it.
    #[test]
    fn a_headers_last_line_has_no_proportional_extra() {
        let body_below = |header: &str| {
            let bytes = format!(
                r#"<office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" office:mimetype="application/vnd.oasis.opendocument.text">
                <office:styles><style:style style:name="H" style:family="paragraph"><style:paragraph-properties fo:line-height="150%" fo:margin-top="0cm" fo:margin-bottom="0cm"/></style:style></office:styles>
                <office:automatic-styles><style:page-layout style:name="pm"><style:page-layout-properties fo:page-width="21cm" fo:page-height="29.7cm" fo:margin-top="2cm" fo:margin-bottom="2cm" fo:margin-left="2cm" fo:margin-right="2cm"/><style:header-style><style:header-footer-properties fo:min-height="0cm" fo:margin-bottom="0cm"/></style:header-style></style:page-layout></office:automatic-styles>
                <office:master-styles><style:master-page style:name="Standard" style:page-layout-name="pm"><style:header>{header}</style:header></style:master-page></office:master-styles>
                <office:body><office:text><text:p>body</text:p></office:text></office:body></office:document>"#
            );
            let app = App::new();
            app.open_bytes("h.fodt", bytes.as_bytes()).unwrap();
            let texts = texts(&typeset(&app, &setter(), &Options::default()).pages[0]);
            let y = |w: &str| texts.iter().find(|t| t.2 == w).unwrap().1;
            (
                y("body") - y("A"),
                texts.iter().find(|t| t.2 == "B").map(|t| t.1 - y("A")),
            )
        };
        let (one, _) = body_below(r#"<text:p text:style-name="H">A</text:p>"#);
        let (two, between) = body_below(
            r#"<text:p text:style-name="H">A</text:p><text:p text:style-name="H">B</text:p>"#,
        );
        let between = between.expect("both lines");
        assert!((between / one - 1.5).abs() < 0.02, "{between} vs {one}");
        assert!((two - between - one).abs() < 0.01, "{two}");
    }

    /// `doc/odt-format.md` §5c fact 68: the margin below a header's last paragraph is not given;
    /// below one before it, it is.
    #[test]
    fn a_headers_last_margin_below_is_not_given() {
        let gap = |header: &str| {
            let bytes = format!(
                r#"<office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" office:mimetype="application/vnd.oasis.opendocument.text">
                <office:styles>
                  <style:style style:name="Plain" style:family="paragraph"><style:paragraph-properties fo:margin-top="0cm" fo:margin-bottom="0cm"/></style:style>
                  <style:style style:name="Below" style:family="paragraph"><style:paragraph-properties fo:margin-top="0cm" fo:margin-bottom="1cm"/></style:style>
                </office:styles>
                <office:automatic-styles><style:page-layout style:name="pm"><style:page-layout-properties fo:page-width="21cm" fo:page-height="29.7cm" fo:margin-top="2cm" fo:margin-bottom="2cm" fo:margin-left="2cm" fo:margin-right="2cm"/><style:header-style><style:header-footer-properties fo:min-height="0cm" fo:margin-bottom="0cm"/></style:header-style></style:page-layout></office:automatic-styles>
                <office:master-styles><style:master-page style:name="Standard" style:page-layout-name="pm"><style:header>{header}</style:header></style:master-page></office:master-styles>
                <office:body><office:text><text:p text:style-name="Plain">body</text:p></office:text></office:body></office:document>"#
            );
            let app = App::new();
            app.open_bytes("h.fodt", bytes.as_bytes()).unwrap();
            let texts = texts(&typeset(&app, &setter(), &Options::default()).pages[0]);
            let y = |w: &str| texts.iter().find(|t| t.2 == w).unwrap().1;
            y("body") - y("A")
        };
        let cm = (72.0 / 2.54) as f32;
        let plain = gap(r#"<text:p text:style-name="Plain">A</text:p>"#);
        let last = gap(r#"<text:p text:style-name="Below">A</text:p>"#);
        let first = gap(
            r#"<text:p text:style-name="Below">A</text:p><text:p text:style-name="Plain">B</text:p>"#,
        );
        assert!((last - plain).abs() < 0.01, "{last} vs {plain}");
        assert!((first - 2.0 * plain - cm).abs() < 0.05, "{first}");
    }

    /// `doc/odt-format.md` §5c fact 46: with `AddParaTableSpacing` off, two paragraphs are the
    /// larger of their spaces apart rather than both; with it on, both.
    #[test]
    fn add_para_table_spacing_off_keeps_the_larger_space() {
        let gap = |setting: &str| {
            let bytes = format!(
                r#"<office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" xmlns:config="urn:oasis:names:tc:opendocument:xmlns:config:1.0" office:mimetype="application/vnd.oasis.opendocument.text">
                <office:settings><config:config-item-set config:name="ooo:configuration-settings"><config:config-item config:name="AddParaTableSpacing" config:type="boolean">{setting}</config:config-item></config:config-item-set></office:settings>
                <office:styles>
                  <style:style style:name="A" style:family="paragraph"><style:paragraph-properties fo:margin-bottom="1cm"/></style:style>
                  <style:style style:name="B" style:family="paragraph"><style:paragraph-properties fo:margin-top="0.5cm"/></style:style>
                </office:styles>
                <office:body><office:text><text:p text:style-name="A">one</text:p><text:p text:style-name="B">two</text:p></office:text></office:body></office:document>"#
            );
            let app = App::new();
            app.open_bytes("s.fodt", bytes.as_bytes()).unwrap();
            let texts = texts(&typeset(&app, &setter(), &Options::default()).pages[0]);
            texts[1].1 - texts[0].1
        };
        let cm = (72.0 / 2.54) as f32;
        let (both, larger) = (gap("true"), gap("false"));
        assert!(
            (both - larger - 0.5 * cm).abs() < 0.01,
            "{both} and {larger}"
        );
    }

    /// `doc/odt-format.md` §5c fact 45: a style stating the automatic font colour prints black
    /// under a parent stating a colour.
    #[test]
    fn a_styles_automatic_colour_replaces_its_parents() {
        let bytes = r##"<office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" office:mimetype="application/vnd.oasis.opendocument.text">
            <office:styles>
              <style:style style:name="Heading" style:family="paragraph"><style:text-properties fo:color="#007c3c"/></style:style>
              <style:style style:name="Subtitle" style:family="paragraph" style:parent-style-name="Heading"><style:text-properties style:use-window-font-color="true"/></style:style>
            </office:styles>
            <office:body><office:text><text:p text:style-name="Heading">green</text:p><text:p text:style-name="Subtitle">black</text:p></office:text></office:body></office:document>"##;
        let app = App::new();
        app.open_bytes("c.fodt", bytes.as_bytes()).unwrap();
        let page = &typeset(&app, &setter(), &Options::default()).pages[0];
        let colour = |want: &str| {
            page.ops.iter().find_map(|op| match op {
                Op::Text { text, color, .. } if text == want => Some(*color),
                _ => None,
            })
        };
        assert_eq!(colour("green"), Some(Rgb(0, 0x7c, 0x3c)));
        assert_eq!(colour("black"), Some(Rgb::BLACK));
    }

    /// A paragraph style's colour colours its text where the run states none, and a link with no
    /// colour of its own is drawn as Writer draws an unstyled one: navy and underlined.
    #[test]
    fn a_styles_colour_and_a_links_look_are_drawn() {
        let bytes = r##"<office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" xmlns:xlink="http://www.w3.org/1999/xlink" office:mimetype="application/vnd.oasis.opendocument.text">
            <office:styles><style:style style:name="Teal" style:family="paragraph"><style:text-properties fo:color="#174a5b"/></style:style></office:styles>
            <office:body><office:text>
              <text:p text:style-name="Teal">teal</text:p>
              <text:p>see <text:a xlink:href="https://example.invalid/">there</text:a></text:p>
            </office:text></office:body></office:document>"##;
        let app = App::new();
        app.open_bytes("c.fodt", bytes.as_bytes()).unwrap();
        let page = &typeset(&app, &setter(), &Options::default()).pages[0];
        let colour = |want: &str| {
            page.ops.iter().find_map(|op| match op {
                Op::Text { text, color, .. } if text == want => Some(*color),
                _ => None,
            })
        };
        assert_eq!(colour("teal"), Some(Rgb(0x17, 0x4a, 0x5b)));
        assert_eq!(colour("there"), Some(Rgb(0, 0, 0x80)));
        assert!(page.ops.iter().any(|op| matches!(
            op,
            Op::Line {
                color: Rgb(0, 0, 0x80),
                ..
            }
        )));
        assert_eq!(colour("see "), Some(Rgb::BLACK));
    }

    /// `doc/odt-format.md` §5c fact 39: a link's own character style is drawn — here red and
    /// not underlined.
    #[test]
    fn a_links_own_style_wins_over_the_unstyled_look() {
        let bytes = r##"<office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" xmlns:xlink="http://www.w3.org/1999/xlink" office:mimetype="application/vnd.oasis.opendocument.text">
            <office:styles><style:style style:name="Plain_20_link" style:family="text"><style:text-properties fo:color="#ff0000" style:text-underline-style="none"/></style:style></office:styles>
            <office:body><office:text>
              <text:p>see <text:a xlink:href="https://example.invalid/" text:style-name="Plain_20_link">there</text:a></text:p>
            </office:text></office:body></office:document>"##;
        let app = App::new();
        app.open_bytes("l.fodt", bytes.as_bytes()).unwrap();
        let page = &typeset(&app, &setter(), &Options::default()).pages[0];
        let colour = page.ops.iter().find_map(|op| match op {
            Op::Text { text, color, .. } if text == "there" => Some(*color),
            _ => None,
        });
        assert_eq!(colour, Some(Rgb(0xff, 0, 0)));
        assert!(!page.ops.iter().any(|op| matches!(op, Op::Line { .. })));
    }

    /// A table drawn as its document says: its own column widths, each cell's fill, borders and
    /// padding, and its heading row again at the top of every page it continues on.
    #[test]
    fn a_table_prints_its_own_look_and_repeats_its_heading() {
        let rows: String = (1..=80)
            .fold(String::new(), |mut joined, i| {
let _ = write!(joined, r#"<table:table-row><table:table-cell table:style-name="Body"><text:p>row {i}</text:p></table:table-cell><table:table-cell><text:p>b</text:p></table:table-cell></table:table-row>"#); joined });
        let bytes = format!(
            r##"<office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" office:mimetype="application/vnd.oasis.opendocument.text">
            <office:styles>
              <style:style style:name="Head" style:family="table-cell"><style:table-cell-properties fo:border="0.5pt solid #174a5b" fo:padding="0.07in" fo:background-color="#287271"/></style:style>
              <style:style style:name="Body" style:family="table-cell"><style:table-cell-properties fo:border="0.5pt solid #8da5a5" fo:padding="0.06in"/></style:style>
            </office:styles>
            <office:automatic-styles><style:style style:name="W" style:family="table-column"><style:table-column-properties style:column-width="2in"/></style:style></office:automatic-styles>
            <office:body><office:text><table:table table:name="T"><table:table-column table:style-name="W"/><table:table-column/>
              <table:table-header-rows><table:table-row><table:table-cell table:style-name="Head"><text:p>Case</text:p></table:table-cell><table:table-cell table:style-name="Head"><text:p>Notes</text:p></table:table-cell></table:table-row></table:table-header-rows>
              {rows}
            </table:table></office:text></office:body></office:document>"##
        );
        let app = App::new();
        app.open_bytes("t.fodt", bytes.as_bytes()).unwrap();
        let doc = typeset(&app, &setter(), &Options::default());
        assert!(doc.pages.len() >= 2);
        for page in &doc.pages {
            let first = texts(page)
                .into_iter()
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .unwrap();
            assert_eq!(first.2, "Case", "every page starts with the heading");
        }
        let page = &doc.pages[0];
        assert!(
            page.ops.iter().any(|op| matches!(
                op,
                Op::Rect {
                    color: Rgb(0x28, 0x72, 0x71),
                    ..
                }
            )),
            "the heading's fill"
        );
        assert!(
            page.ops.iter().any(|op| matches!(
                op,
                Op::Line {
                    color: Rgb(0x8d, 0xa5, 0xa5),
                    ..
                }
            )),
            "a body cell's border"
        );
        // The second column starts where the first's two inches end, plus its own padding.
        let notes = texts(page).into_iter().find(|t| t.2 == "Notes").unwrap();
        let expected = 56.6929 + 144.0 + 0.07 * 72.0;
        assert!((notes.0 - expected as f32).abs() < 0.01, "{}", notes.0);
        // The repeated heading is decoration, not text read twice.
        let repeated = doc.pages[1].ops.iter().find_map(|op| match op {
            Op::Text { text, mark, .. } if text == "Case" => Some(*mark),
            _ => None,
        });
        assert_eq!(repeated, Some(crate::ops::Mark::Decoration));
    }

    /// The flat document every test below builds on: `styles` in `office:styles`, `body` in
    /// `office:text`, Liberation Serif 12 pt throughout.
    fn styled(styles: &str, body: &str) -> App {
        styled_with(styles, "", body)
    }

    /// A hanging indent in a table cell — Word's way of setting a title beside a logo — starts
    /// its first line at the cell's edge, as it does in the body.
    #[test]
    fn a_first_line_indent_applies_in_a_table_cell() {
        let app = styled(
            r#"<style:style style:name="Hang" style:family="paragraph"><style:paragraph-properties fo:margin-left="3cm" fo:text-indent="-3cm"/></style:style>"#,
            r#"<table:table table:name="T"><table:table-column/><table:table-row><table:table-cell>
                 <text:p text:style-name="Hang">TITLE</text:p>
               </table:table-cell></table:table-row></table:table>"#,
        );
        let doc = typeset(&app, &setter(), &Options::default());
        let margin = 72.0 * 2.0 / 2.54;
        let title = texts(&doc.pages[0])
            .into_iter()
            .find(|t| t.2 == "TITLE")
            .unwrap();
        assert!(
            title.0 - margin < 10.0,
            "{} from the margin",
            title.0 - margin
        );
    }

    /// A row's stated height is its least: an empty row of a form is as tall as its style says.
    #[test]
    fn a_row_is_at_least_as_tall_as_its_style_says() {
        let app = styled_with(
            "",
            r#"<style:style style:name="Tall" style:family="table-row"><style:table-row-properties style:min-row-height="3cm"/></style:style>"#,
            r#"<table:table table:name="T"><table:table-column/>
                 <table:table-row table:style-name="Tall"><table:table-cell><text:p>a</text:p></table:table-cell></table:table-row>
                 <table:table-row><table:table-cell><text:p>b</text:p></table:table-cell></table:table-row>
               </table:table>"#,
        );
        let doc = typeset(&app, &setter(), &Options::default());
        let at = |want: &str| {
            texts(&doc.pages[0])
                .into_iter()
                .find(|t| t.2 == want)
                .unwrap()
                .1
        };
        let apart = at("b") - at("a");
        assert!((apart - 72.0 * 3.0 / 2.54).abs() < 1.0, "{apart}");
    }

    /// [`styled`], with `automatic` in `office:automatic-styles`.
    fn styled_with(styles: &str, automatic: &str, body: &str) -> App {
        let bytes = format!(
            r#"<office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" office:mimetype="application/vnd.oasis.opendocument.text">
            <office:styles>
              <style:default-style style:family="paragraph"><style:text-properties fo:font-family="'Liberation Serif'" fo:font-size="12pt"/></style:default-style>
              <style:style style:name="Standard" style:family="paragraph"/>
              {styles}
            </office:styles>
            <office:automatic-styles>{automatic}</office:automatic-styles>
            <office:body><office:text>{body}</office:text></office:body></office:document>"#
        );
        let app = App::new();
        app.open_bytes("t.fodt", bytes.as_bytes()).unwrap();
        app
    }

    /// Where each text op starts down the page, by its text, in document order.
    fn baselines(app: &App) -> Vec<(String, f32)> {
        let doc = typeset(app, &setter(), &Options::default());
        let mut out: Vec<(String, f32)> = texts(&doc.pages[0])
            .into_iter()
            .map(|t| (t.2, t.1))
            .collect();
        out.sort_by(|a, b| a.1.total_cmp(&b.1));
        out
    }

    /// `doc/odt-format.md` §5c fact 18: a row is its top border, padding, content and padding;
    /// the last row adds its bottom border; nothing is added after the table; and a cell's
    /// paragraphs keep their own margins. One line of Liberation Serif 12 pt is 13.8 pt.
    #[test]
    fn a_tables_borders_take_room_down_the_page_as_writer_gives_them() {
        let app = styled(
            r#"<style:style style:name="C" style:family="table-cell"><style:table-cell-properties fo:padding="0.1cm" fo:border="6pt solid #000000"/></style:style>
               <style:style style:name="M" style:family="paragraph"><style:paragraph-properties fo:margin-top="0.5cm" fo:margin-bottom="0.3cm"/></style:style>"#,
            r#"<text:p text:style-name="Standard">a</text:p>
               <table:table table:name="T"><table:table-column/>
                 <table:table-row><table:table-cell table:style-name="C"><text:p text:style-name="Standard">b</text:p></table:table-cell></table:table-row>
                 <table:table-row><table:table-cell table:style-name="C"><text:p text:style-name="M">c</text:p></table:table-cell></table:table-row>
               </table:table>
               <text:p text:style-name="Standard">d</text:p>"#,
        );
        let at = baselines(&app);
        let names: Vec<&str> = at.iter().map(|(t, _)| t.as_str()).collect();
        assert_eq!(names, ["a", "b", "c", "d"]);
        let (line, pad, border) = (13.8, 72.0 / 25.4, 6.0);
        let (cm5, cm3) = (5.0 * 72.0 / 25.4, 3.0 * 72.0 / 25.4);
        let gaps = [
            line + border + pad,
            line + pad + border + pad + cm5,
            line + cm3 + pad + border,
        ];
        for (pair, expected) in at.windows(2).zip(gaps) {
            let got = pair[1].1 - pair[0].1;
            assert!(
                (got - expected as f32).abs() < 0.05,
                "{} to {}: {got} for {expected}",
                pair[0].0,
                pair[1].0
            );
        }
    }

    /// A `double` border is two lines, its three `border-line-width`s together as thick as it
    /// is, the outer width outermost across the top.
    #[test]
    fn a_double_border_is_two_lines() {
        let app = styled(
            r#"<style:style style:name="C" style:family="table-cell"><style:table-cell-properties fo:border-top="6pt double #ff0000" style:border-line-width-top="0.05cm 0.05cm 0.1cm"/></style:style>"#,
            r#"<table:table table:name="T"><table:table-column/><table:table-row><table:table-cell table:style-name="C"><text:p>x</text:p></table:table-cell></table:table-row></table:table>"#,
        );
        let doc = typeset(&app, &setter(), &Options::default());
        let mut lines: Vec<(f32, f32)> = doc.pages[0]
            .ops
            .iter()
            .filter_map(|op| match op {
                Op::Line {
                    from,
                    width,
                    color: Rgb(0xff, 0, 0),
                    ..
                } => Some((from.1, *width)),
                _ => None,
            })
            .collect();
        lines.sort_by(|a, b| a.0.total_cmp(&b.0));
        let pt = |cm: f32| cm * 72.0 / 2.54;
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert!((lines[0].1 - pt(0.1)).abs() < 1e-3, "the outer line on top");
        assert!((lines[1].1 - pt(0.05)).abs() < 1e-3);
        assert!(((lines[1].0 - lines[0].0) - pt(0.05 + 0.05 + 0.025)).abs() < 1e-3);
    }

    /// `doc/odt-format.md` §5c fact 19: a paragraph's trailing spaces count when it is aligned.
    #[test]
    fn a_paragraphs_trailing_spaces_count_when_it_is_aligned() {
        let app = styled(
            r#"<style:style style:name="C" style:family="paragraph"><style:paragraph-properties fo:text-align="center"/></style:style>
               <style:style style:name="E" style:family="paragraph"><style:paragraph-properties fo:text-align="end"/></style:style>"#,
            r#"<text:p text:style-name="C">X</text:p><text:p text:style-name="C">X </text:p>
               <text:p text:style-name="E">X</text:p><text:p text:style-name="E">X </text:p>"#,
        );
        let doc = typeset(&app, &setter(), &Options::default());
        let mut at = texts(&doc.pages[0]);
        at.sort_by(|a, b| a.1.total_cmp(&b.1));
        // A Liberation Serif space at 12 pt is 3 pt.
        assert!((at[0].0 - at[1].0 - 1.5).abs() < 0.01, "{at:?}");
        assert!((at[2].0 - at[3].0 - 3.0).abs() < 0.01, "{at:?}");
    }

    /// `doc/odt-format.md` §5c facts 20 and 21: a heading's number and its text, where its
    /// outline level puts them — the positions are the ones Writer printed, from the page's
    /// text edge.
    #[test]
    fn a_heading_wears_its_outline_number_where_its_level_puts_it() {
        let level = |n: u32, attrs: &str, align: &str| {
            format!(
                r#"<text:outline-level-style text:level="{n}" {attrs}><style:list-level-properties text:list-level-position-and-space-mode="label-alignment"><style:list-level-label-alignment {align}/></style:list-level-properties></text:outline-level-style>"#
            )
        };
        let outline = [
            level(1, r#"style:num-format="1""#, r#"text:label-followed-by="listtab" text:list-tab-stop-position="1cm" fo:text-indent="-1cm" fo:margin-left="1cm""#),
            level(2, r#"style:num-format="1" text:display-levels="2" style:num-suffix="....""#, r#"text:label-followed-by="listtab" text:list-tab-stop-position="0.3cm" fo:text-indent="-0.3cm" fo:margin-left="0.3cm""#),
            level(3, r#"style:num-format="A" xmlns:loext="urn:org:documentfoundation:names:experimental:office:xmlns:loext:1.0" loext:num-list-format="(%1%.%2%.%3%)""#, r#"text:label-followed-by="space" fo:text-indent="0cm" fo:margin-left="2cm""#),
            level(5, r#"style:num-format="""#, r#"text:label-followed-by="listtab" text:list-tab-stop-position="1cm" fo:text-indent="-1cm" fo:margin-left="1cm""#),
        ]
        .concat();
        let app = styled(
            &format!(
                r#"<style:style style:name="H" style:family="paragraph"/><text:outline-style style:name="Outline">{outline}</text:outline-style>"#
            ),
            r#"<text:h text:style-name="H" text:outline-level="1">Aa</text:h>
               <text:h text:style-name="H" text:outline-level="2">Bb</text:h>
               <text:h text:style-name="H" text:outline-level="3">Cc</text:h>
               <text:h text:style-name="H" text:outline-level="5">Ee</text:h>
               <text:h text:style-name="H" text:outline-level="1" text:is-list-header="true">Hh</text:h>
               <text:h text:style-name="H" text:outline-level="1">Ii</text:h>"#,
        );
        let doc = typeset(&app, &setter(), &Options::default());
        let margin = 72.0 * 2.0 / 2.54;
        let at: Vec<(String, f32)> = texts(&doc.pages[0])
            .into_iter()
            .map(|t| (t.2, ((t.0 - margin) * 100.0).round() / 100.0))
            .collect();
        let expected = [
            ("1", 0.0),
            ("Aa", 28.35),
            ("1.1....", 0.0),
            ("Bb", 43.94),
            ("(1.1.A)", 56.69),
            ("Cc", 94.35),
            ("Ee", 0.0),
            ("Hh", 28.35),
            ("2", 0.0),
            ("Ii", 28.35),
        ];
        assert_eq!(at.len(), expected.len(), "{at:?}");
        for ((text, x), (want, wx)) in at.iter().zip(expected) {
            assert_eq!(text, want);
            assert!((x - wx).abs() < 0.05, "{text} at {x}, Writer's at {wx}");
        }
    }

    /// `doc/odt-format.md` §5c fact 23: a heading inside a list wears the list's number at the
    /// list's depth, counted with its paragraphs, and the outline neither numbers nor counts it.
    #[test]
    fn a_heading_in_a_list_is_numbered_by_the_list() {
        let placed = r#"<style:list-level-properties text:list-level-position-and-space-mode="label-alignment"><style:list-level-label-alignment text:label-followed-by="listtab" text:list-tab-stop-position="1cm" fo:text-indent="-1cm" fo:margin-left="1cm"/></style:list-level-properties>"#;
        let app = styled(
            &format!(
                r#"<style:style style:name="H" style:family="paragraph"/>
                   <text:outline-style style:name="Outline">
                     <text:outline-level-style text:level="1" style:num-format="I" style:num-suffix=")">{placed}</text:outline-level-style>
                     <text:outline-level-style text:level="2" style:num-format="A" style:num-suffix=")">{placed}</text:outline-level-style>
                   </text:outline-style>
                   <text:list-style style:name="L1">
                     <text:list-level-style-number text:level="1" style:num-format="1" style:num-suffix=".">{placed}</text:list-level-style-number>
                     <text:list-level-style-number text:level="2" style:num-format="a" style:num-suffix=")">{placed}</text:list-level-style-number>
                   </text:list-style>"#
            ),
            r#"<text:h text:outline-level="1">OUTA</text:h>
               <text:list xml:id="x1" text:style-name="L1">
                 <text:list-item><text:h text:outline-level="1">LISTH1</text:h></text:list-item>
                 <text:list-item><text:p>LISTP2</text:p></text:list-item>
                 <text:list-item><text:h text:outline-level="2">LISTH3</text:h></text:list-item>
                 <text:list-item><text:list><text:list-item><text:h text:outline-level="1">NESTH</text:h></text:list-item></text:list></text:list-item>
               </text:list>
               <text:h text:outline-level="1">OUTB</text:h>
               <text:list text:continue-list="x1" text:style-name="L1">
                 <text:list-item><text:h text:outline-level="1">LISTH4</text:h></text:list-item>
               </text:list>
               <text:list><text:list-item><text:h text:outline-level="1">NOSTYLE</text:h></text:list-item></text:list>
               <text:h text:outline-level="2">OUTC</text:h>"#,
        );
        let doc = typeset(&app, &setter(), &Options::default());
        let shown: Vec<String> = texts(&doc.pages[0]).into_iter().map(|t| t.2).collect();
        assert_eq!(
            shown,
            [
                "I)", "OUTA", "1.", "LISTH1", "2.", "LISTP2", "3.", "LISTH3", "a)", "NESTH", "II)",
                "OUTB", "4.", "LISTH4", "NOSTYLE", "A)", "OUTC"
            ]
        );
    }

    /// `doc/odt-format.md` §5c fact 44: a stretched background covers the paper, and one not
    /// repeated sits at its stated size in the paper's bottom-right corner — both behind the text.
    #[test]
    fn a_pages_background_picture_is_drawn_behind_it() {
        let layout = |name: &str, fill: &str| {
            format!(
                r#"<style:page-layout style:name="{name}"><style:page-layout-properties fo:page-width="21cm" fo:page-height="29.7cm" fo:margin-top="2cm" fo:margin-bottom="2cm" fo:margin-left="2cm" fo:margin-right="2cm" draw:fill="bitmap" {fill}><style:background-image><office:binary-data>iVBORw0KGgo=</office:binary-data></style:background-image></style:page-layout-properties></style:page-layout>"#
            )
        };
        let bytes = format!(
            r#"<office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0" office:mimetype="application/vnd.oasis.opendocument.text">
            <office:automatic-styles>
              <style:style style:name="OnCorner" style:family="paragraph" style:master-page-name="Corner"/>
              {cover}{corner}
            </office:automatic-styles>
            <office:master-styles>
              <style:master-page style:name="Standard" style:page-layout-name="cover"/>
              <style:master-page style:name="Corner" style:page-layout-name="corner"/>
            </office:master-styles>
            <office:body><office:text><text:p>cover</text:p><text:p text:style-name="OnCorner">corner</text:p></office:text></office:body></office:document>"#,
            cover = layout("cover", r#"style:repeat="stretch""#),
            corner = layout(
                "corner",
                r#"style:repeat="no-repeat" draw:fill-image-width="10cm" draw:fill-image-height="5cm" draw:fill-image-ref-point="bottom-right""#
            ),
        );
        let app = App::new();
        app.open_bytes("bg.fodt", bytes.as_bytes()).unwrap();
        let doc = typeset(&app, &setter(), &Options::default());
        assert_eq!(doc.pages.len(), 2);
        let cm = (72.0 / 2.54) as f32;
        let first = |page: &crate::ops::Page| match page.ops.first() {
            Some(Op::Image {
                x,
                y,
                width,
                height,
                ..
            }) => (*x, *y, *width, *height),
            other => panic!("{other:?}"),
        };
        let (w, h) = (doc.pages[0].width, doc.pages[0].height);
        let near = |a: (f32, f32, f32, f32), b: (f32, f32, f32, f32)| {
            [a.0 - b.0, a.1 - b.1, a.2 - b.2, a.3 - b.3]
                .iter()
                .all(|d| d.abs() < 0.01)
        };
        assert!(near(first(&doc.pages[0]), (0.0, 0.0, w, h)));
        let corner = first(&doc.pages[1]);
        assert!(
            near(corner, (w - 10.0 * cm, h - 5.0 * cm, 10.0 * cm, 5.0 * cm)),
            "{corner:?}"
        );
    }

    /// `doc/odt-format.md` §5c fact 43: a footer's chapter field — here in an `as-char` frame's
    /// text box, as the Calc Guide sets it — shows the last level-1 heading begun by its page.
    #[test]
    fn a_footers_chapter_field_shows_the_pages_chapter() {
        let bytes = r#"<office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0" office:mimetype="application/vnd.oasis.opendocument.text">
            <office:automatic-styles>
              <style:style style:name="Break" style:family="paragraph"><style:paragraph-properties fo:break-before="page"/></style:style>
              <style:page-layout style:name="pm"><style:page-layout-properties fo:page-width="21cm" fo:page-height="29.7cm" fo:margin-top="2cm" fo:margin-bottom="2cm" fo:margin-left="2cm" fo:margin-right="2cm"/></style:page-layout>
            </office:automatic-styles>
            <office:master-styles><style:master-page style:name="Standard" style:page-layout-name="pm"><style:footer>
              <text:p><draw:frame text:anchor-type="as-char"><draw:text-box><text:p><text:chapter text:display="name" text:outline-level="1">cached</text:chapter></text:p></draw:text-box></draw:frame> | <text:page-number/></text:p>
            </style:footer></style:master-page></office:master-styles>
            <office:body><office:text>
              <text:p>before</text:p><text:h text:outline-level="1">Alpha<text:line-break/>Beta</text:h><text:p>one</text:p>
              <text:p text:style-name="Break">two</text:p><text:h text:outline-level="1">Gamma</text:h><text:p>three</text:p><text:h text:outline-level="1">Delta</text:h>
            </office:text></office:body></office:document>"#;
        let app = App::new();
        app.open_bytes("chapter.fodt", bytes.as_bytes()).unwrap();
        let doc = typeset(&app, &setter(), &Options::default());
        let footer = |page: usize| {
            let texts = texts(&doc.pages[page]);
            let bottom = texts.iter().map(|t| t.1).fold(0.0, f32::max);
            texts
                .iter()
                .filter(|t| (t.1 - bottom).abs() < 0.01)
                .map(|t| t.2.clone())
                .collect::<String>()
        };
        assert_eq!(footer(0), "Alpha Beta | 1");
        assert_eq!(footer(1), "Delta | 2");
    }

    /// `doc/odt-format.md` §5c fact 42: a named style that names the list style itself places
    /// its items by its own margin and indent; one that does not is placed by the level.
    #[test]
    fn a_style_naming_its_list_style_places_the_item() {
        let app = styled_with(
            r#"<style:style style:name="Listed" style:family="paragraph" style:list-style-name="L"><style:paragraph-properties fo:margin-left="5cm" fo:text-indent="0cm"/></style:style>
               <style:style style:name="Named" style:family="paragraph"><style:paragraph-properties fo:margin-left="5cm" fo:text-indent="0cm"/></style:style>
               <text:list-style style:name="L">
                 <text:list-level-style-number text:level="1" style:num-suffix="." style:num-format="1"><style:list-level-properties text:list-level-position-and-space-mode="label-alignment"><style:list-level-label-alignment text:label-followed-by="listtab" text:list-tab-stop-position="2cm" fo:text-indent="-1cm" fo:margin-left="2cm"/></style:list-level-properties></text:list-level-style-number>
               </text:list-style>"#,
            "",
            r#"<text:list text:style-name="L"><text:list-item><text:p text:style-name="Listed">Aa</text:p></text:list-item></text:list>
               <text:list text:style-name="L"><text:list-item><text:p text:style-name="Named">Bb</text:p></text:list-item></text:list>"#,
        );
        let doc = typeset(&app, &setter(), &Options::default());
        let margin = 72.0 * 2.0 / 2.54;
        let cm = |cm: f32| cm * 72.0 / 2.54;
        let labels: Vec<f32> = texts(&doc.pages[0])
            .into_iter()
            .filter(|t| t.2 == "1.")
            .map(|t| t.0 - margin)
            .collect();
        assert_eq!(labels.len(), 2);
        assert!((labels[0] - cm(5.0)).abs() < 0.05, "{labels:?}");
        assert!((labels[1] - cm(1.0)).abs() < 0.05, "{labels:?}");
    }

    /// `doc/odt-format.md` §5c fact 60: in the older mode the level's room adds to the
    /// paragraph's own margin and first-line indent, and a wide label pushes the text along.
    #[test]
    fn an_older_level_adds_to_the_paragraphs_margins() {
        let level = |name: &str, prefix: &str| {
            format!(
                r#"<text:list-style style:name="{name}"><text:list-level-style-number text:level="1" style:num-prefix="{prefix}" style:num-suffix="." style:num-format="1"><style:list-level-properties text:space-before="1cm" text:min-label-width="0.8cm" text:min-label-distance="0.3cm"/></text:list-level-style-number></text:list-style>"#
            )
        };
        let words = "words enough to wrap onto a second line of the paragraph and further on, and on and on again until it wraps";
        let item = |list: &str, style: &str, first: &str| {
            format!(
                r#"<text:list text:style-name="{list}"><text:list-item><text:p text:style-name="{style}">{first} {words}</text:p></text:list-item></text:list>"#
            )
        };
        let app = styled_with(
            &format!("{}{}", level("L", ""), level("W", "Article ")),
            r#"<style:style style:name="P" style:family="paragraph" style:parent-style-name="Standard"><style:paragraph-properties fo:margin-left="1cm" fo:text-indent="-0.5cm"/></style:style>"#,
            &[
                item("L", "Standard", "Alpha"),
                item("L", "P", "Charlie"),
                item("W", "Standard", "Delta"),
            ]
            .concat(),
        );
        let doc = typeset(&app, &setter(), &Options::default());
        let margin = 72.0 * 2.0 / 2.54;
        let cm = |at: f32| (at - margin) * 2.54 / 72.0;
        let texts = texts(&doc.pages[0]);
        let at = |start: &str| {
            texts
                .iter()
                .filter(|t| t.2.starts_with(start))
                .map(|t| cm(t.0))
                .collect::<Vec<_>>()
        };
        let near = |got: &[f32], want: &[f32]| {
            got.len() == want.len() && got.iter().zip(want).all(|(a, b)| (a - b).abs() < 0.05)
        };
        assert!(near(&at("1."), &[1.0, 1.5]), "{texts:?}");
        assert!(near(&at("Alpha"), &[1.8]), "{:?}", at("Alpha"));
        assert!(near(&at("Charlie"), &[2.3]), "{:?}", at("Charlie"));
        assert!(near(&at("Article"), &[1.0]), "{:?}", at("Article"));
        // The label ends 0.3 cm short of where the text starts.
        let delta = at("Delta")[0];
        assert!(delta > 2.6 && delta < 3.2, "{delta}");
        // Every later line at the margin plus the level's room.
        let later: Vec<f32> = texts
            .iter()
            .filter(|t| {
                !["1.", "Alpha", "Charlie", "Article", "Delta"]
                    .iter()
                    .any(|s| t.2.starts_with(s))
            })
            .map(|t| cm(t.0))
            .collect();
        assert!(near(&later, &[1.8, 2.8, 1.8]), "{later:?}");
    }

    /// `doc/odt-format.md` §5c fact 48: a label aligned to its end or centre at the indent, and
    /// the text after its tab at the stop past it, else the margin past it, else a default stop.
    #[test]
    fn a_labels_tab_goes_to_the_stop_then_the_margin_then_a_default() {
        let level = |name: &str, align: &str, margin: &str, indent: &str, tab: &str| {
            format!(
                r#"<text:list-style style:name="{name}"><text:list-level-style-number text:level="1" style:num-suffix=")" style:num-format="1"><style:list-level-properties text:list-level-position-and-space-mode="label-alignment" fo:text-align="{align}"><style:list-level-label-alignment text:label-followed-by="listtab" text:list-tab-stop-position="{tab}" fo:text-indent="{indent}" fo:margin-left="{margin}"/></style:list-level-properties></text:list-level-style-number></text:list-style>"#
            )
        };
        let styles = [
            level("E", "end", "1.5cm", "-0.25cm", "0.25cm"),
            level("S", "start", "1.5cm", "-0.25cm", "0.25cm"),
            level("C", "start", "3cm", "-1cm", "0.25cm"),
            level("D", "start", "3cm", "-1cm", "4cm"),
            level("G", "center", "1.5cm", "-0.25cm", "0.25cm"),
        ]
        .concat();
        let body: String = ["E", "S", "C", "D", "G"]
            .iter()
            .fold(String::new(), |mut joined, n| {
let _ = write!(joined, r#"<text:list text:style-name="{n}"><text:list-item><text:p text:style-name="Standard">{n}x</text:p></text:list-item></text:list>"#); joined });
        let app = styled_with(&styles, "", &body);
        let doc = typeset(&app, &setter(), &Options::default());
        let margin = 72.0 * 2.0 / 2.54;
        let cm = |cm: f32| cm * 72.0 / 2.54;
        let texts = texts(&doc.pages[0]);
        let x = |word: &str| texts.iter().find(|t| t.2 == word).unwrap().0 - margin;
        let labels: Vec<f32> = texts
            .iter()
            .filter(|t| t.2 == "1)")
            .map(|t| t.0 - margin)
            .collect();
        assert_eq!(labels.len(), 5, "{texts:?}");
        let label = cm(0.38);
        for (at, want) in [
            (0, cm(1.25) - label),
            (1, cm(1.25)),
            (2, cm(2.0)),
            (3, cm(2.0)),
            (4, cm(1.25) - label / 2.0),
        ] {
            assert!(
                (labels[at] - want).abs() < 1.0,
                "label {at} at {}, Writer's {want}",
                labels[at]
            );
        }
        for (word, want) in [
            ("Ex", 1.5),
            ("Sx", 2.75),
            ("Cx", 3.0),
            ("Dx", 4.0),
            ("Gx", 1.5),
        ] {
            assert!(
                (x(word) - cm(want)).abs() < 0.05,
                "{word} at {}, Writer's {}",
                x(word),
                cm(want)
            );
        }
    }

    /// `doc/odt-format.md` §5c fact 22: a list item's label and text where its level puts
    /// them, an automatic style's own indents winning and a named style's not.
    #[test]
    fn a_list_item_sits_where_its_level_puts_it() {
        let app = styled_with(
            r#"<style:style style:name="Named" style:family="paragraph"><style:paragraph-properties fo:margin-left="4cm" fo:text-indent="0cm"/></style:style>
               <text:list-style style:name="L">
                 <text:list-level-style-number text:level="1" style:num-suffix="." style:num-format="1"><style:list-level-properties text:list-level-position-and-space-mode="label-alignment"><style:list-level-label-alignment text:label-followed-by="listtab" text:list-tab-stop-position="1.27cm" fo:text-indent="-0.635cm" fo:margin-left="1.27cm"/></style:list-level-properties></text:list-level-style-number>
                 <text:list-level-style-bullet text:level="2" text:bullet-char="•"><style:list-level-properties text:list-level-position-and-space-mode="label-alignment"><style:list-level-label-alignment text:label-followed-by="listtab" text:list-tab-stop-position="1.905cm" fo:text-indent="-0.635cm" fo:margin-left="1.905cm"/></style:list-level-properties></text:list-level-style-bullet>
               </text:list-style>"#,
            r#"<style:style style:name="P1" style:family="paragraph" style:parent-style-name="Standard"><style:paragraph-properties fo:margin-left="3cm" fo:text-indent="-1cm"/></style:style>"#,
            r#"<text:list text:style-name="L">
                 <text:list-item><text:p text:style-name="Standard">Aa</text:p><text:list><text:list-item><text:p text:style-name="Standard">Bb</text:p></text:list-item></text:list></text:list-item>
                 <text:list-item><text:p text:style-name="P1">Cc</text:p></text:list-item>
                 <text:list-item><text:p text:style-name="Named">Dd</text:p></text:list-item>
                 <text:list-item><text:p text:style-name="Standard">Ee</text:p><text:p text:style-name="Standard">Ff</text:p></text:list-item>
               </text:list>"#,
        );
        let doc = typeset(&app, &setter(), &Options::default());
        let margin = 72.0 * 2.0 / 2.54;
        let at: Vec<(String, f32)> = texts(&doc.pages[0])
            .into_iter()
            .map(|t| (t.2, t.0 - margin))
            .collect();
        let cm = |cm: f32| cm * 72.0 / 2.54;
        let expected = [
            ("1.", cm(0.635)),
            ("Aa", cm(1.27)),
            ("•", cm(1.27)),
            ("Bb", cm(1.905)),
            ("2.", cm(2.0)),
            ("Cc", cm(3.0)),
            ("3.", cm(0.635)),
            ("Dd", cm(1.27)),
            ("4.", cm(0.635)),
            ("Ee", cm(1.27)),
            ("Ff", cm(1.27)),
        ];
        assert_eq!(at.len(), expected.len(), "{at:?}");
        for ((text, x), (want, wx)) in at.iter().zip(expected) {
            assert_eq!(text, want);
            assert!((x - wx).abs() < 0.05, "{text} at {x}, Writer's at {wx}");
        }
    }

    #[test]
    fn an_empty_document_is_one_blank_page() {
        let app = doc(&[]);
        let typeset = typeset(&app, &setter(), &Options::default());
        assert_eq!(typeset.pages.len(), 1);
        assert!(
            typeset.pages[0].ops.is_empty(),
            "{:?}",
            typeset.pages[0].ops
        );
        assert_eq!((typeset.pages[0].start, typeset.pages[0].end), (None, None));
    }
    /// A flat document of 12 pt Liberation Sans paragraphs between `Before` and `After`, on 2 cm
    /// margins, with the boxed styles §5c fact 30 was measured with.
    fn boxed(body: &str) -> App {
        let bytes = format!(
            r##"<office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" office:mimetype="application/vnd.oasis.opendocument.text">
            <office:styles><style:style style:name="Standard" style:family="paragraph"><style:paragraph-properties fo:margin-top="0cm" fo:margin-bottom="0cm" fo:line-height="100%"/><style:text-properties fo:font-size="12pt" fo:font-family="'Liberation Sans'"/></style:style></office:styles>
            <office:automatic-styles>
              <style:style style:name="B" style:family="paragraph" style:parent-style-name="Standard"><style:paragraph-properties fo:border="2pt solid #ff0000" fo:padding="0.2cm"/></style:style>
              <style:style style:name="BM" style:family="paragraph" style:parent-style-name="Standard"><style:paragraph-properties fo:border="2pt solid #ff0000" fo:padding="0.2cm" fo:margin-left="1cm" fo:margin-right="2cm" fo:margin-top="0.5cm" fo:margin-bottom="0.5cm"/></style:style>
              <style:style style:name="BX" style:family="paragraph" style:parent-style-name="B"><style:paragraph-properties fo:margin-top="0.3cm" fo:background-color="#0000ff"/></style:style>
              <style:style style:name="BNJ" style:family="paragraph" style:parent-style-name="B"><style:paragraph-properties style:join-border="false"/></style:style>
              <style:style style:name="G" style:family="paragraph" style:parent-style-name="Standard"><style:paragraph-properties fo:background-color="#0000ff" fo:margin-left="1cm"/></style:style>
              <style:style style:name="Break" style:family="paragraph" style:parent-style-name="Standard"><style:paragraph-properties fo:break-before="page" fo:margin-top="0.5cm"/></style:style>
              <style:page-layout style:name="pm"><style:page-layout-properties fo:page-width="21cm" fo:page-height="29.7cm" fo:margin-left="2cm" fo:margin-right="2cm" fo:margin-top="2cm" fo:margin-bottom="2cm"/></style:page-layout>
            </office:automatic-styles>
            <office:master-styles><style:master-page style:name="Standard" style:page-layout-name="pm"/></office:master-styles>
            <office:body><office:text><text:p text:style-name="Standard">Before</text:p>{body}<text:p text:style-name="Standard">After</text:p></office:text></office:body></office:document>"##
        );
        let app = App::new();
        app.open_bytes("boxed.fodt", bytes.as_bytes()).unwrap();
        app
    }

    type Stroke = ((f32, f32), (f32, f32), f32);

    /// The red strokes on a page, as `(from, to, width)`, and its blue fills as `(x, y, w, h)`.
    fn strokes_and_fills(page: &crate::ops::Page) -> (Vec<Stroke>, Vec<[f32; 4]>) {
        let mut strokes = Vec::new();
        let mut fills = Vec::new();
        for op in &page.ops {
            match op {
                Op::Line {
                    from,
                    to,
                    width,
                    color,
                } if *color == Rgb(0xff, 0, 0) => strokes.push((*from, *to, *width)),
                Op::Rect {
                    x,
                    y,
                    width,
                    height,
                    color,
                } if *color == Rgb(0, 0, 0xff) => fills.push([*x, *y, *width, *height]),
                _ => {}
            }
        }
        (strokes, fills)
    }

    const CM: f32 = (72.0 / 2.54) as f32;
    const LINE: f32 = 13.8;

    fn near(got: f32, want: f32, what: &str) {
        assert!((got - want).abs() < 0.3, "{what}: {got} vs {want}");
    }

    /// §5c fact 30: borders and padding take room inside the margins, and the box is closed.
    #[test]
    fn a_bordered_paragraph_is_boxed_inside_its_margins() {
        let doc = typeset(
            &boxed(r#"<text:p text:style-name="BM">Boxed</text:p>"#),
            &setter(),
            &Options::default(),
        );
        let page = &doc.pages[0];
        let texts = texts(page);
        let at = |want: &str| texts.iter().find(|t| t.2 == want).expect(want).clone();
        let (before, boxed, after) = (at("Before"), at("Boxed"), at("After"));
        let inset = 2.0 + 0.2 * CM;
        near(
            boxed.0,
            3.0 * CM + inset,
            "set in by its margin, border and padding",
        );
        near(
            boxed.1 - before.1,
            LINE + 0.5 * CM + inset,
            "down by its margin, border and padding",
        );
        near(
            after.1 - boxed.1,
            LINE + inset + 0.5 * CM,
            "the next below the box and the margin",
        );
        let (strokes, _) = strokes_and_fills(page);
        assert_eq!(strokes.len(), 4, "a closed box: {strokes:?}");
        let top = 2.0 * CM + LINE + 0.5 * CM;
        let xs = strokes.iter().flat_map(|s| [s.0.0, s.1.0]);
        let ys = strokes.iter().flat_map(|s| [s.0.1, s.1.1]);
        near(
            xs.clone().fold(f32::MAX, f32::min),
            3.0 * CM,
            "the box's left",
        );
        near(xs.fold(f32::MIN, f32::max), 17.0 * CM, "the box's right");
        near(ys.clone().fold(f32::MAX, f32::min), top, "the box's top");
        near(
            ys.fold(f32::MIN, f32::max),
            top + LINE + 2.0 * inset,
            "the box's bottom",
        );
    }

    /// §5c fact 30: equal neighbours share one box, the margins between them inside it, and
    /// `style:join-border="false"` or a different margin keeps them apart.
    #[test]
    fn equal_bordered_paragraphs_join_into_one_box() {
        let inset = 2.0 + 0.2 * CM;
        let horizontal = |doc: &Document| {
            let (strokes, fills) = strokes_and_fills(&doc.pages[0]);
            let mut ys: Vec<f32> = strokes
                .iter()
                .filter(|s| (s.0.1 - s.1.1).abs() < 1e-3)
                .map(|s| s.0.1)
                .collect();
            ys.sort_by(f32::total_cmp);
            (ys, fills)
        };
        let joined = typeset(
            &boxed(
                r#"<text:p text:style-name="B">First</text:p><text:p text:style-name="BX">Second</text:p>"#,
            ),
            &setter(),
            &Options::default(),
        );
        let texts = texts(&joined.pages[0]);
        let at = |want: &str| texts.iter().find(|t| t.2 == want).expect(want).clone();
        near(
            at("Second").1 - at("First").1,
            LINE + 0.3 * CM,
            "no border between them, the margin inside",
        );
        let (ys, fills) = horizontal(&joined);
        assert_eq!(ys.len(), 2, "one top and one bottom: {ys:?}");
        near(
            ys[1] - ys[0],
            2.0 * LINE + 0.3 * CM + 2.0 * inset - 2.0,
            "one box round both",
        );
        // The second's fill covers the box it would have on its own, reaching into the first's.
        assert_eq!(fills.len(), 1, "{fills:?}");
        near(fills[0][2], 17.0 * CM, "across the box");
        let ascent = at("First").1 - (2.0 * CM + LINE + inset);
        near(
            fills[0][1],
            at("Second").1 - ascent - inset,
            "its own top inset above its line",
        );
        near(fills[0][3], LINE + 2.0 * inset, "its line and both insets");

        let apart =
            |body: &str| horizontal(&typeset(&boxed(body), &setter(), &Options::default())).0;
        let ys = apart(
            r#"<text:p text:style-name="BNJ">First</text:p><text:p text:style-name="BNJ">Second</text:p>"#,
        );
        assert_eq!(ys.len(), 4, "two closed boxes: {ys:?}");
        let ys = apart(
            r#"<text:p text:style-name="B">First</text:p><text:p text:style-name="BM">Second</text:p>"#,
        );
        assert_eq!(ys.len(), 4, "a different margin, two boxes: {ys:?}");
    }

    /// §5c fact 30: a background with no border or padding covers the margins' span and the line.
    #[test]
    fn a_paragraphs_background_covers_its_box() {
        let doc = typeset(
            &boxed(r#"<text:p text:style-name="G">Shaded</text:p>"#),
            &setter(),
            &Options::default(),
        );
        let (strokes, fills) = strokes_and_fills(&doc.pages[0]);
        assert!(strokes.is_empty(), "{strokes:?}");
        assert_eq!(fills.len(), 1);
        let [x, y, w, h] = fills[0];
        near(x, 3.0 * CM, "from the left margin");
        near(w, 16.0 * CM, "to the text area's right");
        near(y, 2.0 * CM + LINE, "the paragraph's top");
        near(h, LINE, "one line");
    }
    /// §5c fact 31: no space between neighbours of the same named style where contextual spacing
    /// says so — the first's below by its own flag, the second's above by both.
    #[test]
    fn contextual_spacing_drops_the_space_between_paragraphs_of_one_style() {
        let gaps = |body: &str| -> Vec<f32> {
            let bytes = format!(
                r#"<office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" office:mimetype="application/vnd.oasis.opendocument.text">
                <office:styles>
                  <style:style style:name="Standard" style:family="paragraph"><style:paragraph-properties fo:margin-top="0cm" fo:margin-bottom="0cm" fo:line-height="100%"/><style:text-properties fo:font-size="12pt" fo:font-family="'Liberation Sans'"/></style:style>
                  <style:style style:name="C" style:family="paragraph" style:parent-style-name="Standard"><style:paragraph-properties fo:margin-top="0.5cm" fo:margin-bottom="1cm" style:contextual-spacing="true"/></style:style>
                  <style:style style:name="D" style:family="paragraph" style:parent-style-name="Standard"><style:paragraph-properties fo:margin-top="0.5cm" fo:margin-bottom="1cm" style:contextual-spacing="true"/></style:style>
                </office:styles>
                <office:automatic-styles>
                  <style:style style:name="P1" style:family="paragraph" style:parent-style-name="C"><style:paragraph-properties fo:text-align="end"/></style:style>
                  <style:style style:name="Off" style:family="paragraph" style:parent-style-name="C"><style:paragraph-properties fo:margin-top="0.5cm" fo:margin-bottom="1cm" style:contextual-spacing="false"/></style:style>
                </office:automatic-styles>
                <office:body><office:text><text:p text:style-name="Standard">Before</text:p>{body}<text:p text:style-name="Standard">After</text:p></office:text></office:body></office:document>"#
            );
            let app = App::new();
            app.open_bytes("cs.fodt", bytes.as_bytes()).unwrap();
            let doc = typeset(&app, &setter(), &Options::default());
            let ys: Vec<f32> = texts(&doc.pages[0]).iter().map(|t| t.1).collect();
            ys.array_windows()
                .map(|[prev, next]| (next - prev - LINE) / CM)
                .collect()
        };
        let p = |style: &str| format!(r#"<text:p text:style-name="{style}">x</text:p>"#);
        let near_all = |got: Vec<f32>, want: &[f32], what: &str| {
            assert_eq!(got.len(), want.len(), "{what}: {got:?}");
            for (g, w) in got.iter().zip(want) {
                assert!((g - w).abs() < 0.02, "{what}: {got:?} vs {want:?}");
            }
        };
        near_all(
            gaps(&(p("C") + &p("C") + &p("P1"))),
            &[0.5, 0.0, 0.0, 1.0],
            "one style",
        );
        near_all(gaps(&(p("C") + &p("D"))), &[0.5, 1.5, 1.0], "two styles");
        near_all(
            gaps(&(p("C") + &p("Off"))),
            &[0.5, 0.5, 1.0],
            "the first's flag",
        );
        near_all(
            gaps(&(p("Off") + &p("C"))),
            &[0.5, 1.5, 1.0],
            "the second's alone",
        );
    }
    /// §5c fact 33: a cell's text at the top, halfway down, or with its last line on the bottom
    /// padding, as `style:vertical-align` says.
    #[test]
    fn a_cells_text_sits_down_its_row_as_its_style_says() {
        let bytes = r#"<office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" office:version="1.4" office:mimetype="application/vnd.oasis.opendocument.text">
<office:styles><style:style style:name="Standard" style:family="paragraph"><style:paragraph-properties fo:margin-top="0cm" fo:margin-bottom="0cm" fo:line-height="100%"/><style:text-properties fo:font-size="12pt" fo:font-family="'Liberation Sans'"/></style:style></office:styles>
<office:automatic-styles>
<style:style style:name="R" style:family="table-row"><style:table-row-properties style:min-row-height="3cm"/></style:style>
<style:style style:name="CT" style:family="table-cell"><style:table-cell-properties style:vertical-align="top" fo:padding="0.2cm" fo:border="0.5pt solid #000000"/></style:style>
<style:style style:name="CM" style:family="table-cell"><style:table-cell-properties style:vertical-align="middle" fo:padding="0.2cm" fo:border="0.5pt solid #000000"/></style:style>
<style:style style:name="CB" style:family="table-cell"><style:table-cell-properties style:vertical-align="bottom" fo:padding="0.2cm" fo:border="0.5pt solid #000000"/></style:style>
<style:page-layout style:name="pm"><style:page-layout-properties fo:page-width="21cm" fo:page-height="29.7cm" fo:margin-left="2cm" fo:margin-right="2cm" fo:margin-top="2cm" fo:margin-bottom="2cm"/></style:page-layout></office:automatic-styles>
<office:master-styles><style:master-page style:name="Standard" style:page-layout-name="pm"/></office:master-styles>
<office:body><office:text><table:table table:name="T"><table:table-column table:number-columns-repeated="3"/><table:table-row table:style-name="R">
<table:table-cell table:style-name="CT"><text:p text:style-name="Standard">Top</text:p></table:table-cell>
<table:table-cell table:style-name="CM"><text:p text:style-name="Standard">Mid</text:p></table:table-cell>
<table:table-cell table:style-name="CB"><text:p text:style-name="Standard">Bot</text:p><text:p text:style-name="Standard">Two</text:p></table:table-cell>
</table:table-row></table:table><text:p text:style-name="Standard">After</text:p></office:text></office:body></office:document>"#;
        let app = App::new();
        app.open_bytes("va.fodt", bytes.as_bytes()).unwrap();
        let doc = typeset(&app, &setter(), &Options::default());
        let texts = texts(&doc.pages[0]);
        let at = |want: &str| texts.iter().find(|t| t.2 == want).expect(want).1;
        // A 3 cm row, 0.2 cm padding and a 0.5 pt border above and below: what the text leaves.
        let slack = 3.0 * CM - 0.4 * CM - 1.0 - LINE;
        near(at("Mid") - at("Top"), slack / 2.0, "halfway down");
        near(
            at("Two") - at("Top"),
            slack,
            "the last line on the bottom padding",
        );
        near(
            at("Bot") - at("Top"),
            slack - LINE,
            "the first a line above it",
        );
    }
    /// §5c fact 34: at the top of a page a box keeps its border and padding, and after a page
    /// break a paragraph keeps its margin too.
    #[test]
    fn a_paragraph_starting_a_page_keeps_its_box_and_after_a_break_its_margin() {
        let lines: String = (0..51).fold(String::new(), |mut joined, i| {
            let _ = write!(
                joined,
                r#"<text:p text:style-name="Standard">line{i}</text:p>"#
            );
            joined
        });
        let inset = 2.0 + 0.2 * CM;
        let flowed = typeset(
            &boxed(&format!(
                r#"{lines}<text:p text:style-name="B">Boxed</text:p>"#
            )),
            &setter(),
            &Options::default(),
        );
        assert_eq!(flowed.pages.len(), 2);
        let page = &flowed.pages[1];
        let (strokes, _) = strokes_and_fills(page);
        let top = strokes
            .iter()
            .flat_map(|s| [s.0.1, s.1.1])
            .fold(f32::MAX, f32::min);
        near(top, 2.0 * CM, "the box on the top margin");
        let opening = texts(page)[0].clone();
        assert_eq!(opening.2, "Boxed");
        let forced = typeset(
            &boxed(r#"<text:p text:style-name="Break">Broken</text:p>"#),
            &setter(),
            &Options::default(),
        );
        let first = texts(&flowed.pages[0])[0].1;
        let broken = texts(&forced.pages[1])[0].1;
        near(broken - first, 0.5 * CM, "its margin kept after the break");
        near(
            opening.1 - first,
            inset,
            "its border and padding kept at the top",
        );
    }
}
