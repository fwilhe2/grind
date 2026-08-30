// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The command palette's vocabulary — every verb either pane can do, as data.
//!
//! **This shell's answer to a menu bar.** A browser tab has no menu bar to hang things off
//! and no window manager to borrow one from, and inventing a pretend one is how a web
//! application ends up looking like a worse copy of a desktop application. So the verbs live
//! in one searchable list that Ctrl+K opens, the toolbar carries only what a *pointer* wants
//! (the toggles and the swatches), and nothing is reachable from one and not the other.
//!
//! It is data rather than closures on purpose: a [`Command`] is an `id` and some words, the
//! pane matches on the id, and the whole list — including "does every command have a home" —
//! unit-tests on the host with no browser, exactly as the keymaps do.
//!
//! The palette is also where **going somewhere** lives. A pane contributes [`Entry`] values of
//! its own for whatever the query looks like — an address, a sheet's name, a heading — so the
//! same box that runs *Bold* also jumps to `B12`, to `Summary`, or to the heading called
//! *Method*. One box, because a second one would be a second thing to find.

/// One verb, as the palette shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Command {
    /// What the pane matches on. Stable, lower-case, `group.verb` — never shown.
    pub id: &'static str,
    pub title: &'static str,
    pub group: &'static str,
    /// The shortcut, spelled for a reader. Empty when there is none, which is most of them:
    /// the palette *is* the shortcut.
    pub keys: &'static str,
    /// Whether it shows before anything has been typed. The list a reader meets should be
    /// short enough to read; everything else is one letter away.
    pub common: bool,
}

/// A row in the palette: a [`Command`], or something a pane made up for this query.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub id: String,
    pub title: String,
    pub group: String,
    pub keys: String,
    /// Which characters of `title` the query matched, for the emphasis the palette draws.
    pub hits: Vec<usize>,
}

impl Entry {
    /// A pane's own entry — a place to go rather than a verb to run.
    pub fn target(id: impl Into<String>, title: impl Into<String>, group: &str) -> Self {
        Entry {
            id: id.into(),
            title: title.into(),
            group: group.to_owned(),
            keys: String::new(),
            hits: Vec::new(),
        }
    }
}

/// The spreadsheet's verbs.
///
/// Every one of these reaches an `App` method, and every one is also on the toolbar or a key
/// — the palette adds a *way in*, never a capability the other shells do not have.
pub const SHEET: &[Command] = &[
    // --- what a document does ---
    cmd("doc.open", "Open document…", "Document", "Ctrl+O", true),
    cmd("doc.save", "Save a copy", "Document", "Ctrl+S", true),
    cmd("doc.undo", "Undo", "Document", "Ctrl+Z", false),
    cmd("doc.redo", "Redo", "Document", "Ctrl+Shift+Z", false),
    cmd("sheet.recalc", "Recalculate", "Document", "F9", true),
    // --- the selection ---
    cmd("edit.copy", "Copy", "Edit", "Ctrl+C", true),
    cmd("edit.cut", "Cut", "Edit", "Ctrl+X", false),
    cmd("edit.paste", "Paste", "Edit", "Ctrl+V", false),
    cmd("edit.clear", "Clear contents", "Edit", "Delete", false),
    cmd("edit.fill-down", "Fill down", "Edit", "Ctrl+D", true),
    cmd("edit.fill-right", "Fill right", "Edit", "Ctrl+R", false),
    cmd(
        "edit.select-all",
        "Select the whole sheet",
        "Edit",
        "Ctrl+A",
        false,
    ),
    // --- how it looks ---
    cmd("style.bold", "Bold", "Format", "Ctrl+B", true),
    cmd("style.italic", "Italic", "Format", "Ctrl+I", true),
    cmd("style.align-left", "Align left", "Format", "", false),
    cmd("style.align-center", "Align centre", "Format", "", false),
    cmd("style.align-right", "Align right", "Format", "", false),
    cmd(
        "style.align-clear",
        "Align automatically",
        "Format",
        "",
        false,
    ),
    cmd("style.wrap", "Wrap text", "Format", "", false),
    cmd("style.border", "Add borders", "Format", "", false),
    cmd("style.border-clear", "Remove borders", "Format", "", false),
    cmd("style.clear", "Clear formatting", "Format", "", true),
    // --- what it means ---
    cmd("format.general", "Number: general", "Number", "", true),
    cmd("format.number", "Number: 1 234.57", "Number", "", false),
    cmd("format.integer", "Number: 1 235", "Number", "", false),
    cmd("format.percent", "Number: per cent", "Number", "", false),
    cmd("format.currency", "Number: currency", "Number", "", false),
    cmd("format.date", "Number: date", "Number", "", false),
    cmd(
        "format.datetime",
        "Number: date and time",
        "Number",
        "",
        false,
    ),
    cmd("format.time", "Number: time", "Number", "", false),
    cmd("format.more", "More decimal places", "Number", "", false),
    cmd("format.fewer", "Fewer decimal places", "Number", "", false),
    // --- what it is, rather than what it says ---
    //
    // `doc/view-modes.md`. Neither writes anything: they are readings of the document, and
    // the same command turns one off, which is why they sit beside the verbs rather than
    // behind a confirmation.
    cmd("view.roles", "Show what each cell is", "View", "", true),
    cmd("view.names", "Show where names live", "View", "", false),
    // `doc/dsl.md` §6. The same command turns it off, like the two above, and for the same
    // reason: it is a *reading* of the document and writes nothing.
    cmd("view.source", "Show the source", "View", "", false),
    // --- the workbook ---
    cmd("sheet.add", "Add a sheet", "Sheets", "", true),
    cmd("sheet.rename", "Rename this sheet…", "Sheets", "", false),
    cmd("sheet.delete", "Delete this sheet", "Sheets", "", false),
];

/// The word processor's verbs.
pub const TEXT: &[Command] = &[
    cmd("doc.open", "Open document…", "Document", "Ctrl+O", true),
    cmd("doc.save", "Save a copy", "Document", "Ctrl+S", true),
    cmd("doc.undo", "Undo", "Document", "Ctrl+Z", false),
    cmd("doc.redo", "Redo", "Document", "Ctrl+Shift+Z", false),
    cmd("edit.copy", "Copy", "Edit", "Ctrl+C", true),
    cmd("edit.cut", "Cut", "Edit", "Ctrl+X", false),
    cmd("edit.paste", "Paste", "Edit", "Ctrl+V", false),
    cmd(
        "edit.select-all",
        "Select everything",
        "Edit",
        "Ctrl+A",
        false,
    ),
    cmd("char.bold", "Bold", "Format", "Ctrl+B", true),
    cmd("char.italic", "Italic", "Format", "Ctrl+I", true),
    cmd("char.underline", "Underline", "Format", "Ctrl+U", true),
    cmd("char.strike", "Strikethrough", "Format", "", false),
    cmd("char.clear", "Clear formatting", "Format", "", true),
    cmd("block.body", "Body text", "Structure", "Ctrl+0", true),
    cmd("block.h1", "Heading 1", "Structure", "Ctrl+1", true),
    cmd("block.h2", "Heading 2", "Structure", "Ctrl+2", true),
    cmd("block.h3", "Heading 3", "Structure", "Ctrl+3", false),
    cmd("block.h4", "Heading 4", "Structure", "", false),
    cmd("block.title", "Title", "Structure", "", false),
    cmd("block.subtitle", "Subtitle", "Structure", "", false),
    cmd("block.list", "List item", "Structure", "", false),
    cmd(
        "block.indent",
        "Indent this list item",
        "Structure",
        "Tab",
        false,
    ),
    cmd(
        "block.outdent",
        "Outdent this list item",
        "Structure",
        "Shift+Tab",
        false,
    ),
    // §3.6, the word processor's half of inline names: a bookmark is the one part of a text
    // document a reader cannot see at all, because it contributes no characters.
    cmd("view.names", "Show where bookmarks are", "View", "", false),
    cmd("view.source", "Show the source", "View", "", false),
];

/// `const fn` so the tables above stay readable — a struct literal per row is the same
/// information three times as tall.
const fn cmd(
    id: &'static str,
    title: &'static str,
    group: &'static str,
    keys: &'static str,
    common: bool,
) -> Command {
    Command {
        id,
        title,
        group,
        keys,
        common,
    }
}

/// The commands that match `needle`, best first — or, for an empty query, the common ones in
/// the order they are declared.
///
/// The rank is deliberately simple and deliberately *stable*: a palette whose first row moves
/// around as a fourth letter is typed is a palette nobody trusts to press Enter on.
pub fn filter(commands: &[Command], needle: &str) -> Vec<Entry> {
    let needle = needle.trim();
    if needle.is_empty() {
        return commands
            .iter()
            .filter(|command| command.common)
            .map(|command| entry(command, Vec::new()))
            .collect();
    }
    let mut scored: Vec<(i32, usize, Entry)> = commands
        .iter()
        .enumerate()
        .filter_map(|(order, command)| {
            // The group is searchable too, so "number" finds every number format even though
            // none of their titles start with it.
            let haystack = format!("{} {}", command.title, command.group);
            let (score, hits) = score(&haystack, needle)?;
            // Only the hits inside the title can be drawn; the group's are off the end.
            let hits = hits
                .into_iter()
                .filter(|at| *at < command.title.chars().count())
                .collect();
            Some((score, order, entry(command, hits)))
        })
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    scored.into_iter().map(|(_, _, entry)| entry).collect()
}

fn entry(command: &Command, hits: Vec<usize>) -> Entry {
    Entry {
        id: command.id.to_owned(),
        title: command.title.to_owned(),
        group: command.group.to_owned(),
        keys: command.keys.to_owned(),
        hits,
    }
}

/// How well `needle` matches `haystack`, and where — a subsequence match, case-insensitive,
/// scoring a letter that starts a word above one in the middle of one and a run of adjacent
/// letters above a scattering. `None` when a letter of the needle is not there at all.
///
/// Positions are in `char`s, because that is what the palette slices the title by.
pub fn score(haystack: &str, needle: &str) -> Option<(i32, Vec<usize>)> {
    let hay: Vec<char> = haystack.chars().flat_map(char::to_lowercase).collect();
    // Word starts, computed once: the beginning, and any letter after a space or a hyphen.
    let boundary = |at: usize| at == 0 || matches!(hay.get(at - 1), Some(' ' | '-' | ':' | '('));

    let mut score = 0;
    let mut hits = Vec::new();
    let mut at = 0;
    let mut previous: Option<usize> = None;
    for want in needle.chars().flat_map(char::to_lowercase) {
        if want == ' ' {
            continue;
        }
        let found = hay[at..].iter().position(|c| *c == want)? + at;
        score += match () {
            // `checked_sub`, because the first character of the haystack is position zero and
            // `found - 1` there is not a position at all.
            _ if found.checked_sub(1) == previous && previous.is_some() => 6,
            _ if boundary(found) => 5,
            _ => 1,
        };
        hits.push(found);
        previous = Some(found);
        at = found + 1;
    }
    // A short title that matched is a better answer than a long one that also did: "Bold"
    // should beat "Bold the selection's borders" for `bol`.
    score = score * 100 - hay.len() as i32;
    Some((score, hits))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_query_shows_the_common_commands_in_order() {
        let shown = filter(SHEET, "");
        assert!(!shown.is_empty());
        assert!(shown.iter().all(|entry| entry.hits.is_empty()));
        // Declaration order, not alphabetical: the first thing offered is opening a document.
        assert_eq!(shown[0].id, "doc.open");
        let common = SHEET.iter().filter(|c| c.common).count();
        assert_eq!(shown.len(), common);
    }

    #[test]
    fn a_query_searches_everything_including_the_uncommon() {
        let shown = filter(SHEET, "border");
        let ids: Vec<&str> = shown.iter().map(|e| e.id.as_str()).collect();
        assert!(ids.contains(&"style.border"), "{ids:?}");
        assert!(ids.contains(&"style.border-clear"), "{ids:?}");
    }

    /// The group is part of the haystack, so a word that appears in no title still finds the
    /// family it names.
    #[test]
    fn searching_by_group_finds_the_whole_group() {
        let ids: Vec<String> = filter(SHEET, "number").into_iter().map(|e| e.id).collect();
        assert!(ids.iter().any(|id| id == "format.percent"), "{ids:?}");
        assert!(ids.iter().any(|id| id == "format.date"), "{ids:?}");
    }

    #[test]
    fn a_letter_at_the_start_of_a_word_beats_one_in_the_middle() {
        let (start, _) = score("Align left", "al").unwrap();
        let (middle, _) = score("Recalculate", "al").unwrap();
        assert!(start > middle, "{start} vs {middle}");
    }

    #[test]
    fn the_exact_word_comes_first() {
        assert_eq!(filter(SHEET, "bold")[0].id, "style.bold");
        assert_eq!(filter(SHEET, "paste")[0].id, "edit.paste");
        assert_eq!(filter(TEXT, "heading 2")[0].id, "block.h2");
    }

    #[test]
    fn a_letter_that_is_not_there_matches_nothing() {
        assert_eq!(score("Bold", "bz"), None);
        assert!(filter(SHEET, "zzzz").is_empty());
    }

    #[test]
    fn the_matched_letters_come_back_so_they_can_be_shown() {
        let (_, hits) = score("Align left", "alle").unwrap();
        assert_eq!(hits, vec![0, 1, 6, 7]);
        // A hit past the end of the title is dropped, or the palette would slice out of range.
        let entry = filter(SHEET, "format")
            .into_iter()
            .find(|e| e.id == "style.bold");
        if let Some(entry) = entry {
            assert!(
                entry
                    .hits
                    .iter()
                    .all(|at| *at < entry.title.chars().count())
            );
        }
    }

    /// Every id has to be unique, or two rows of the palette run the same thing — and the
    /// pane's own `run` match would silently pick one.
    #[test]
    fn no_two_commands_share_an_id() {
        for table in [SHEET, TEXT] {
            let mut ids: Vec<&str> = table.iter().map(|c| c.id).collect();
            ids.sort_unstable();
            let count = ids.len();
            ids.dedup();
            assert_eq!(ids.len(), count, "a duplicate id");
        }
    }
}
