// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `grind_text::App` — the shape every shell drives.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use grind_text::{App, BlockKind, Form, Observer, loc};

/// Build a document *through the App*, which means the setup is itself undoable — every
/// helper below accounts for that rather than pretending history starts empty.
fn app(paragraphs: &[&str]) -> App {
    let app = App::new();
    for (i, text) in paragraphs.iter().enumerate() {
        app.insert(i, BlockKind::Paragraph, text).expect("inserts");
    }
    app
}

/// Undo until there is nothing left, and report how many steps it took.
fn undo_all(app: &App) -> usize {
    let mut steps = 0;
    while app.undo() {
        steps += 1;
    }
    steps
}

fn text(app: &App) -> String {
    app.get_viewport(0..app.block_count())
        .iter()
        .map(|b| b.text.clone())
        .collect::<Vec<_>>()
        .join("\n")
}

/// The test `doc/plan.md` rule 3 exists for, and the reason `App::mutate` drops the write lock
/// before notifying. Without that, this hangs forever rather than failing.
#[test]
fn an_observer_may_read_the_app_without_deadlocking() {
    struct Reader {
        app: Mutex<Option<Arc<App>>>,
        seen: AtomicUsize,
    }
    impl Observer for Reader {
        fn changed(&self) {
            let app = self.app.lock().unwrap().clone().expect("wired up");
            // Straight back in, for a *read*, while the mutation that notified us is on the
            // stack. This is exactly what a shell does when it repaints.
            let _ = app.get_viewport(0..app.block_count());
            self.seen.fetch_add(1, Ordering::SeqCst);
        }
    }

    let app = Arc::new(App::new());
    let observer = Arc::new(Reader {
        app: Mutex::new(Some(app.clone())),
        seen: AtomicUsize::new(0),
    });
    app.set_observer(observer.clone());

    app.insert(0, BlockKind::Paragraph, "hello")
        .expect("inserts");
    assert_eq!(observer.seen.load(Ordering::SeqCst), 1);
}

#[test]
fn a_viewport_is_a_window_and_reading_past_the_end_is_not_an_error() {
    let app = app(&["a", "b", "c"]);
    let view = app.get_viewport(1..2);
    assert_eq!(view.len(), 1);
    assert_eq!(view.get(1).expect("in the window").text, "b");
    assert!(view.get(0).is_none(), "outside the window is None");

    // Scrolling into blank space is normal.
    let past = app.get_viewport(10..20);
    assert!(past.is_empty());
    assert_eq!(app.get_viewport(0..100).len(), 3, "short, not an error");
}

#[test]
fn every_edit_undoes_and_redoes_in_one_step() {
    let app = app(&["one", "two", "three"]);
    let before = text(&app);

    app.set_text(1, "changed").expect("sets");
    app.set_style(0..3, Some("Body".into())).expect("styles");
    app.delete(0..2).expect("deletes");
    assert_eq!(text(&app), "three");

    // Three edits, three undos — a batch is one step, not three. `set_style` over three
    // blocks and `delete` over two are each *one* entry.
    assert!(app.undo() && app.undo() && app.undo());
    assert_eq!(text(&app), before);

    assert!(app.redo() && app.redo() && app.redo());
    assert_eq!(text(&app), "three");

    // And unwinding the whole history — the three edits plus the three inserts that built the
    // document — leaves nothing, which is what says no entry was lost or doubled.
    assert_eq!(undo_all(&app), 6);
    assert_eq!(text(&app), "");
    assert!(!app.can_undo());
}

// --- the caret-level edits (S7) -------------------------------------------------------------
//
// The four operations a continuous-flow editor is made of, and the reason they are in the core
// rather than in a shell: three shells writing their own would disagree about the answers
// below, and `doc/plan.md` rule 4 means the CLI has to be able to do them anyway.

/// Turn an address into a caret the way every caller does — through `loc`, which is the only
/// place a 1-based `p12+40` becomes indices.
fn caret(app: &App, address: &str) -> grind_text::Caret {
    app.resolve_caret(&loc::parse(address).expect("parses"))
        .expect("resolves")
}

#[test]
fn typing_puts_characters_at_a_caret() {
    let app = app(&["hello world"]);
    app.insert_text(caret(&app, "p1+5"), ",").expect("types");
    assert_eq!(text(&app), "hello, world");

    // The two ends, which is where an off-by-one would show.
    app.insert_text(caret(&app, "p1+0"), ">> ").expect("types");
    let end = app.input_text(0).expect("reads").chars().count();
    app.insert_text(
        app.resolve_caret(&loc::parse(&format!("p1+{end}")).unwrap())
            .expect("resolves"),
        "!",
    )
    .expect("types");
    assert_eq!(text(&app), ">> hello, world!");

    // Three keystrokes, three undo steps — typing is not batched, because the granularity a
    // shell wants (a word? a pause?) is a shell's decision and it can only coarsen what the
    // core gives it, never refine it.
    assert!(app.undo() && app.undo() && app.undo());
    assert_eq!(text(&app), "hello world");
}

#[test]
fn an_inserted_image_is_one_caret_position_like_a_tab() {
    let app = app(&["hello world"]);
    app.insert_image(
        caret(&app, "p1+5"),
        "image/png".to_owned(),
        vec![1, 2, 3, 4],
        Some("5cm".to_owned()),
        Some("5cm".to_owned()),
    )
    .expect("inserts the image");

    // The object replacement character stands in for it in plain text, one character wide —
    // what `Block::len` and every caret arithmetic in this crate count against.
    assert_eq!(text(&app), "hello\u{fffc} world");
    assert_eq!(app.input_text(0).expect("reads").chars().count(), 12);

    // Backspace at the position right after it removes it whole, not by however many bytes
    // its data happens to be.
    let after = caret(&app, "p1+6");
    let before = grind_text::Caret {
        block: 0,
        offset: 5,
    };
    app.erase(before, after).expect("erases");
    assert_eq!(text(&app), "hello world");

    assert!(app.undo() && app.undo());
    assert_eq!(text(&app), "hello world");
}

#[test]
fn typed_text_takes_the_formatting_of_the_run_at_the_caret() {
    // "plain" then "bold" in a span, which is what reading `<text:span>` produces.
    let app = App::new();
    app.insert(0, BlockKind::Paragraph, "").expect("inserts");
    let styled = |text: &str, style: Option<&str>| grind_text::Run::Text {
        text: text.to_owned(),
        style: style.map(str::to_owned),
        props: Default::default(),
        href: None,
    };
    let doc = |app: &App| {
        let bytes = app.save_bytes(Form::Flat).expect("saves");
        String::from_utf8(bytes).expect("utf-8")
    };

    // Build the two runs by writing and reading back, so the test drives the real path.
    let mut d = grind_text::Document::new();
    let id = d.next_id();
    let mut block = grind_text::Block::new(id, BlockKind::Paragraph);
    block.runs = vec![styled("plain", None), styled("bold", Some("T1"))];
    d.blocks.push(block);
    let bytes = grind_text::write_bytes(&d, Form::Flat).expect("writes");
    app.open_bytes("doc", &bytes).expect("opens");
    assert_eq!(text(&app), "plainbold");

    // Inside the styled run: styled. `T1` appearing twice would mean the run was fragmented.
    app.insert_text(caret(&app, "p1+7"), "XX").expect("types");
    assert_eq!(text(&app), "plainboXXld");
    assert_eq!(doc(&app).matches("T1").count(), 1, "one span, not three");

    // At the boundary between them the left wins: typing continues what you just typed.
    let app2 = App::new();
    app2.open_bytes("doc", &bytes).expect("opens");
    app2.insert_text(caret(&app2, "p1+5"), "YY").expect("types");
    assert_eq!(app2.formatting().len(), 1);
    assert!(
        doc(&app2).contains("plainYY"),
        "the plain run absorbed it: {}",
        doc(&app2)
    );

    // At the very front there is nothing to the left, so the run to the right decides.
    let app3 = App::new();
    app3.open_bytes("doc", &bytes).expect("opens");
    app3.insert_text(caret(&app3, "p1+0"), "ZZ").expect("types");
    assert!(doc(&app3).contains("ZZplain"), "{}", doc(&app3));
}

#[test]
fn erasing_within_one_block_removes_exactly_the_span() {
    let app = app(&["hello, world"]);
    let (from, to) = app
        .resolve_caret_range(&loc::parse_range("p1+5:p1+7").expect("parses"))
        .expect("resolves");
    assert_eq!(app.erase(from, to).expect("erases"), 2);
    assert_eq!(text(&app), "helloworld");

    // A bare address is the whole block's text — the block stays, empty. `delete` is the verb
    // that removes a block; `erase` only ever removes characters.
    let (from, to) = app
        .resolve_caret_range(&loc::parse_range("p1").expect("parses"))
        .expect("resolves");
    app.erase(from, to).expect("erases");
    assert_eq!(app.block_count(), 1);
    assert_eq!(text(&app), "");

    assert!(app.undo() && app.undo());
    assert_eq!(text(&app), "hello, world");
}

#[test]
fn erasing_across_blocks_leaves_one_and_undoes_in_a_single_step() {
    let app = app(&["first line", "middle", "last line"]);
    app.set_kind(0, BlockKind::Heading { level: 1 }).expect("h");

    let (from, to) = app
        .resolve_caret_range(&loc::parse_range("p1+5:p3+4").expect("parses"))
        .expect("resolves");
    // " line" + "\n" + "middle" + "\n" + "last" — the two closed-up boundaries count one each.
    assert_eq!(app.erase(from, to).expect("erases"), 5 + 1 + 6 + 1 + 4);
    assert_eq!(app.block_count(), 1);
    // "first" and the " line" left at the end of p3, which is a coincidence worth naming so
    // nobody reads it as the erase having done nothing.
    assert_eq!(text(&app), "first line");
    assert_eq!(
        app.get_viewport(0..1).get(0).expect("there").kind,
        BlockKind::Heading { level: 1 },
        "the survivor is the first block, so its kind wins"
    );

    // One step, whatever it took internally — four Ctrl+Z would be the surprise.
    assert!(app.undo());
    assert_eq!(text(&app), "first line\nmiddle\nlast line");
}

#[test]
fn erasing_keeps_an_anchor_it_passes_over_but_not_one_whose_block_is_gone() {
    let app = app(&["hello there", "middle", "tail end"]);
    app.set_bookmark("inner", Some(0)).expect("marks");
    app.set_bookmark("doomed", Some(1)).expect("marks");

    // `inner` is at offset 0 of p1, inside the span being erased: it survives, collapsed to
    // the caret, exactly as `set_text` keeps one through a rewrite.
    let (from, to) = app
        .resolve_caret_range(&loc::parse_range("p1+0:p3+4").expect("parses"))
        .expect("resolves");
    app.erase(from, to).expect("erases");
    assert_eq!(text(&app), " end");

    let names: Vec<String> = app.bookmarks().into_iter().map(|(n, _)| n).collect();
    assert_eq!(
        names,
        vec!["inner".to_owned()],
        "the anchor in a surviving block stays; the one whose block ceased to exist does not, \
         which is what `delete` already does"
    );
}

#[test]
fn splitting_and_joining_are_the_return_and_backspace_keys() {
    let app = app(&["one sentence. two sentence."]);
    app.split_block(caret(&app, "p1+14")).expect("splits");
    assert_eq!(text(&app), "one sentence. \ntwo sentence.");
    assert_eq!(app.block_count(), 2);

    // Joining puts it back, and the first block's kind is the one that survives.
    app.join_block(0).expect("joins");
    assert_eq!(text(&app), "one sentence. two sentence.");
    assert_eq!(app.block_count(), 1);

    // Each is one undo step even though each is two actions.
    assert!(app.undo() && app.undo());
    assert_eq!(text(&app), "one sentence. two sentence.");

    // Nothing follows the last block, so there is nothing to join it to.
    assert!(app.join_block(app.block_count() - 1).is_err());
    assert!(app.split_block(caret(&app, "p1+0")).is_ok(), "at the front");
    assert_eq!(text(&app), "\none sentence. two sentence.");
}

#[test]
fn return_at_the_end_of_a_heading_starts_a_paragraph_and_in_the_middle_does_not() {
    let app = app(&["Chapter One"]);
    app.set_kind(0, BlockKind::Heading { level: 2 }).expect("h");
    app.set_style(0..1, Some("Heading_20_2".into())).expect("s");

    let end = caret(&app, "p1+11");
    app.split_block(end).expect("splits");
    let view = app.get_viewport(0..2);
    assert_eq!(
        view.get(1).expect("there").kind,
        BlockKind::Paragraph,
        "a heading followed by an empty heading is never what anyone meant"
    );
    assert_eq!(
        view.get(1).expect("there").style,
        None,
        "and it does not keep the heading's style either"
    );

    // In the middle it is a title being divided, so both halves stay headings.
    app.undo();
    app.split_block(caret(&app, "p1+8")).expect("splits");
    let view = app.get_viewport(0..2);
    assert_eq!(
        view.get(1).expect("there").kind,
        BlockKind::Heading { level: 2 }
    );
    assert_eq!(
        view.get(1).expect("there").style.as_deref(),
        Some("Heading_20_2")
    );

    // A list item is the other way round: Return at the end of one continues the list.
    let list = self::app(&["item"]);
    list.set_kind(0, BlockKind::ListItem { depth: 2 })
        .expect("li");
    list.split_block(caret(&list, "p1+4")).expect("splits");
    assert_eq!(
        list.get_viewport(0..2).get(1).expect("there").kind,
        BlockKind::ListItem { depth: 2 }
    );
}

#[test]
fn a_caret_edit_past_the_end_is_an_error_rather_than_a_panic() {
    let app = app(&["a"]);
    let past = grind_text::Caret {
        block: 9,
        offset: 0,
    };
    assert!(app.insert_text(past, "x").is_err());
    assert!(app.erase(past, past).is_err());
    assert!(app.split_block(past).is_err());
    assert!(app.join_block(9).is_err());
    // Backwards is refused too, rather than quietly erasing nothing.
    let here = |offset| grind_text::Caret { block: 0, offset };
    assert!(app.erase(here(1), here(0)).is_err());
    assert_eq!(text(&app), "a");
    assert_eq!(undo_all(&app), 1, "no failure reached the undo stack");
}

// --- caret movement by line (doc/text-layout.md, Path C) --------------------------------------
//
// The operations that reopened the layout fork. Every one is defined in terms of a *line*, so
// every one is here rather than in a shell — and every one is answerable with `Fixed`, i.e.
// with no font and no display, which is what makes rule 4 satisfiable.

/// One unit per character, so a width of 10 means ten characters. Every assertion below is
/// exact because of it.
const M: grind_text::Fixed = grind_text::Fixed;

#[test]
fn a_block_wraps_into_lines_at_a_width() {
    let app = app(&["the cat sat on the mat"]);
    let layout = app.layout_block(0, 10.0, &M).expect("lays out");
    assert_eq!(layout.lines().len(), 3);
    assert_eq!(
        layout.lines()[0].end,
        8,
        "\"the cat \" fits, \"the cat sat\" does not"
    );
    // No width means no wrapping, which is what a CLI printing a document plainly asks for.
    assert_eq!(
        app.layout_block(0, 0.0, &M)
            .expect("lays out")
            .lines()
            .len(),
        1
    );
    assert!(app.layout_block(9, 10.0, &M).is_err(), "no such block");
}

/// An empty paragraph is a line of the provider's own height, not of nothing.
///
/// Found by the GTK shell, where the difference is visible: `wrap` takes a line's height
/// from the fragments it was handed, and a block with no runs has none — so the height it
/// fell back to was one unit, which is a correct line in a terminal and a one-pixel gap on a
/// screen. `Tall` below is what makes the difference assertable at all.
#[test]
fn an_empty_block_is_still_one_line_of_the_metrics_own_height() {
    struct Tall;
    impl grind_text::Metrics for Tall {
        fn advances(&self, text: &str, _: &grind_core::style::TextStyle, out: &mut Vec<f32>) {
            for (index, _) in text.chars().enumerate() {
                out.push((index + 1) as f32);
            }
        }
        fn line_height(&self, _: &grind_core::style::TextStyle) -> f32 {
            17.0
        }
    }

    let app = app(&["", "text"]);
    for block in 0..2 {
        let layout = app.layout_block(block, 10.0, &Tall).expect("lays out");
        assert_eq!(layout.lines().len(), 1, "block {block}");
        assert_eq!(layout.height(), 17.0, "block {block}");
    }
}

#[test]
fn down_and_up_move_by_line_and_keep_the_goal_column() {
    let app = app(&["the cat sat on the mat"]);
    let start = caret(&app, "p1+3"); // "the|"
    let goal = app.caret_x(start, 10.0, &M).expect("measures");
    assert_eq!(goal, 3.0);

    let down = app.caret_line(start, 1, goal, 10.0, &M).expect("moves");
    assert_eq!(
        down,
        grind_text::Caret {
            block: 0,
            offset: 11
        },
        "\"sat|\" on line 2"
    );
    let back = app.caret_line(down, -1, goal, 10.0, &M).expect("moves");
    assert_eq!(back, start, "and back to where it came from");

    // Two lines at once — Page Down is the same operation with a bigger number.
    let far = app.caret_line(start, 2, goal, 10.0, &M).expect("moves");
    assert_eq!(app.caret_x(far, 10.0, &M).expect("measures"), 3.0);
}

#[test]
fn down_carries_into_the_next_block_and_stops_at_the_document_edge() {
    let app = app(&["first", "second", "third"]);
    let start = caret(&app, "p1+2");
    let goal = app.caret_x(start, 20.0, &M).expect("measures");

    // Each block is one line at this width, so Down is a block move — the behaviour that makes
    // a document one flow rather than a list of boxes.
    let next = app.caret_line(start, 1, goal, 20.0, &M).expect("moves");
    assert_eq!(
        next,
        grind_text::Caret {
            block: 1,
            offset: 2
        }
    );
    let last = app.caret_line(next, 1, goal, 20.0, &M).expect("moves");
    assert_eq!(last.block, 2);

    // Off the bottom: it stops rather than erroring. A caret that cannot move is not a failure.
    assert_eq!(
        app.caret_line(last, 1, goal, 20.0, &M).expect("stops"),
        last
    );
    let top = grind_text::Caret {
        block: 0,
        offset: 2,
    };
    assert_eq!(app.caret_line(top, -1, goal, 20.0, &M).expect("stops"), top);
}

#[test]
fn home_and_end_are_the_visual_line_not_the_paragraph() {
    let app = app(&["the cat sat on the mat"]);
    // A caret on the middle line: Home and End must give that line's ends, not the block's.
    let at = caret(&app, "p1+11");
    let (home, end) = app.caret_line_bounds(at, 10.0, &M).expect("bounds");
    assert_eq!(home.offset, 8);
    assert_eq!(end.offset, 15);

    // Unwrapped, the same caret's line is the whole block — which is the same code answering a
    // different question, not a special case.
    let (home, end) = app.caret_line_bounds(at, 0.0, &M).expect("bounds");
    assert_eq!((home.offset, end.offset), (0, 22));
}

#[test]
fn a_line_break_run_ends_a_line_even_with_no_width() {
    // `text:line-break` is a mandatory break, so it is one whether or not anything wraps —
    // which is the difference between it and a space, and is why the model keeps it as a run.
    let app = App::new();
    app.insert(0, BlockKind::Paragraph, "one\ntwo")
        .expect("inserts");
    let layout = app.layout_block(0, 0.0, &M).expect("lays out");
    assert_eq!(layout.lines().len(), 2);
    assert_eq!(
        layout.lines()[0].end,
        4,
        "the break stays on the line it ended"
    );
}

#[test]
fn a_paragraph_becomes_a_heading_and_the_outline_follows() {
    let app = app(&["Title", "prose", "Part", "more"]);
    app.set_kind(0, BlockKind::Heading { level: 1 })
        .expect("h1");
    app.set_kind(2, BlockKind::Heading { level: 2 })
        .expect("h2");

    let outline = app.outline();
    assert_eq!(outline.len(), 2);
    assert_eq!(outline[0].address(), "\u{a7}1");
    assert_eq!(outline[1].address(), "\u{a7}1.1");
    assert_eq!(outline[1].text, "Part");

    // And the address resolves back to the block it names.
    let at = loc::parse("\u{a7}1.1").expect("parses");
    assert_eq!(app.resolve(&at).expect("resolves"), 2);
}

#[test]
fn moving_a_section_takes_its_blocks_with_it() {
    let app = app(&["A", "a-text", "B", "b-text", "C"]);
    app.set_kind(0, BlockKind::Heading { level: 1 }).expect("h");
    app.set_kind(2, BlockKind::Heading { level: 1 }).expect("h");
    app.set_kind(4, BlockKind::Heading { level: 1 }).expect("h");

    // Move section B (blocks 2..4) to the very front.
    app.move_blocks(2..4, 0).expect("moves");
    assert_eq!(text(&app), "B\nb-text\nA\na-text\nC");

    // One undo, because a move is one batch.
    assert!(app.undo());
    assert_eq!(text(&app), "A\na-text\nB\nb-text\nC");
}

#[test]
fn a_range_cannot_be_moved_into_its_own_middle() {
    let app = app(&["a", "b", "c", "d"]);
    assert!(app.move_blocks(0..3, 1).is_err());
    assert_eq!(text(&app), "a\nb\nc\nd", "a refused move changes nothing");
}

#[test]
fn a_bookmark_survives_the_paragraph_being_retyped() {
    let app = app(&["intro text", "other"]);
    app.set_bookmark("intro", Some(0)).expect("sets");
    assert_eq!(app.bookmarks(), vec![("intro".to_owned(), 0)]);

    // Rewriting the sentence must not lose the anchor — otherwise `#intro` is useless the
    // first time anyone edits the paragraph it names.
    app.set_text(0, "completely rewritten").expect("sets");
    assert_eq!(app.bookmarks(), vec![("intro".to_owned(), 0)]);
    assert_eq!(app.input_text(0).expect("reads"), "completely rewritten");

    // Setting it again moves it rather than leaving two.
    app.set_bookmark("intro", Some(1)).expect("moves");
    assert_eq!(app.bookmarks(), vec![("intro".to_owned(), 1)]);

    app.set_bookmark("intro", None).expect("removes");
    assert!(app.bookmarks().is_empty());
}

#[test]
fn find_reports_addresses_a_user_can_type_back_in() {
    let app = app(&["the cat sat", "on the mat"]);
    let hits = app.find("the");
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].address(), "p1+0");
    assert_eq!(hits[1].address(), "p2+3");
    assert!(app.find("").is_empty(), "an empty needle matches nothing");
}

#[test]
fn replace_changes_every_occurrence_and_undoes_as_one_step() {
    let app = app(&["a b a", "c", "a"]);
    assert_eq!(
        app.replace("a", "z").expect("replaces"),
        2,
        "blocks changed"
    );
    assert_eq!(text(&app), "z b z\nc\nz");
    assert!(app.undo());
    assert_eq!(text(&app), "a b a\nc\na");
}

#[test]
fn formatting_lists_only_what_carries_a_style_of_its_own() {
    let app = app(&["plain", "styled", "plain too"]);
    assert!(app.formatting().is_empty());

    app.set_style(1..2, Some("Quote".into())).expect("styles");
    let styled = app.formatting();
    assert_eq!(styled.len(), 1);
    assert_eq!(styled[0].text, "styled");
    assert_eq!(styled[0].style.as_deref(), Some("Quote"));
}

#[test]
fn counts_are_what_a_status_bar_shows() {
    let app = app(&["one two three", "four"]);
    app.set_kind(0, BlockKind::Heading { level: 1 }).expect("h");
    let counts = app.counts();
    assert_eq!(counts.words, 4);
    assert_eq!(counts.blocks, 2);
    assert_eq!(counts.headings, 1);
    assert_eq!(counts.characters, "one two three".len() + "four".len());
}

#[test]
fn a_document_survives_being_saved_and_opened() {
    let app = app(&["Title", "body  text", "item"]);
    app.set_kind(0, BlockKind::Heading { level: 1 }).expect("h");
    app.set_kind(2, BlockKind::ListItem { depth: 1 })
        .expect("li");
    app.set_bookmark("here", Some(1)).expect("marks");

    for form in [Form::Flat, Form::Package] {
        let bytes = app.save_bytes(form).expect("saves");
        let back = App::new();
        back.open_bytes("doc", &bytes).expect("opens");
        assert_eq!(text(&back), text(&app), "{form:?}");
        assert_eq!(back.outline().len(), 1, "{form:?}");
        assert_eq!(back.bookmarks().len(), 1, "{form:?}");
    }
}

#[test]
fn opening_a_spreadsheet_is_refused_with_the_command_that_would_work() {
    let app = App::new();
    let ods = br#"<?xml version="1.0"?>
<office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0"
  office:mimetype="application/vnd.oasis.opendocument.spreadsheet"/>"#;
    let err = app.open_bytes("book.ods", ods).expect_err("refused");
    assert!(err.to_string().contains("grind sheet"), "{err}");
}

#[test]
fn an_edit_past_the_end_is_an_error_rather_than_a_panic() {
    let app = app(&["a"]);
    assert!(app.set_text(9, "x").is_err());
    assert!(app.delete(0..9).is_err());
    assert!(app.set_style(0..9, None).is_err());
    assert!(app.set_kind(9, BlockKind::Paragraph).is_err());
    assert_eq!(text(&app), "a");
    // Exactly one entry: the insert that built the document. None of the four failures above
    // pushed anything, which is the property — a refused edit is not half an edit.
    assert_eq!(
        undo_all(&app),
        1,
        "a failed edit must not reach the undo stack"
    );
    assert_eq!(text(&app), "");
}

// --- character formatting -------------------------------------------------------------

fn bold() -> grind_text::CharStyle {
    let mut style = grind_text::CharStyle::default();
    style.set_bold(true);
    style
}

/// The shape a toolbar drives: read what is there, change one field, write the whole thing
/// back. The same contract `grind_sheet::App::set_style` offers, so a shell that has learned
/// one has learned both.
#[test]
fn formatting_a_span_leaves_its_neighbours_alone() {
    let app = app(&["one two three"]);
    let (from, to) = (caret(&app, "p1+4"), caret(&app, "p1+7"));
    assert_eq!(app.set_char_style(from, to, &bold()).expect("styles"), 1);

    assert_eq!(text(&app), "one two three", "formatting is not content");
    assert!(app.char_style(from, to).expect("reads").is_bold());
    assert!(
        !app.char_style(caret(&app, "p1+0"), caret(&app, "p1+3"))
            .expect("reads")
            .is_bold(),
        "the word before it is untouched"
    );
    // Over the whole paragraph nothing is agreed, which is what a toolbar shows for a mixed
    // selection — neither on nor off.
    assert!(
        app.char_style(caret(&app, "p1+0"), caret(&app, "p1+13"))
            .expect("reads")
            .is_plain()
    );

    // Setting *replaces*, so an empty style is "plain again" rather than a no-op.
    assert_eq!(
        app.set_char_style(from, to, &grind_text::CharStyle::default())
            .expect("clears"),
        1
    );
    assert!(app.char_style(from, to).expect("reads").is_plain());
}

/// One `Action::Batch` across however many blocks it touched, so formatting a section is one
/// Ctrl+Z — the property that makes a toolbar usable and a shell unable to get wrong.
#[test]
fn formatting_across_blocks_is_one_undo_step() {
    let app = app(&["first", "second", "third"]);
    let changed = app
        .set_char_style(caret(&app, "p1+2"), caret(&app, "p3+2"), &bold())
        .expect("styles");
    assert_eq!(changed, 3, "every block the span touched");

    assert!(app.undo(), "one step takes all three back");
    for address in ["p1+2", "p2+2", "p3+2"] {
        let at = caret(&app, address);
        assert!(
            app.char_style(at, at).expect("reads").is_plain(),
            "{address}"
        );
    }
    assert_eq!(undo_all(&app), 3, "and the three inserts that built it");
}

/// A caret is an empty span, and what it reports is what the *next keystroke* will look like —
/// the run to its left. Anything else and a toolbar would show one thing while typing produced
/// another.
#[test]
fn a_caret_reports_the_formatting_the_next_keystroke_will_take() {
    let app = app(&["one two"]);
    app.set_char_style(caret(&app, "p1+4"), caret(&app, "p1+7"), &bold())
        .expect("styles");

    let at = |address: &str| {
        let c = caret(&app, address);
        app.char_style(c, c).expect("reads")
    };
    assert!(!at("p1+4").is_bold(), "at the front, the run to the left");
    assert!(at("p1+5").is_bold(), "inside it");
    assert!(at("p1+7").is_bold(), "at the very end, still the left run");

    app.insert_text(caret(&app, "p1+7"), "!").expect("types");
    assert!(
        at("p1+8").is_bold(),
        "so typing at the end of a bold word continues bold"
    );
}

/// The property the whole feature rests on: direct formatting is written as a generated
/// `style:style` and read back as the same properties, in both physical forms.
#[test]
fn direct_formatting_survives_a_save_and_a_load() {
    let app = app(&["one two three"]);
    let mut style = bold();
    style.font_family = Some("Georgia".to_owned());
    style.font_size = Some("14pt".to_owned());
    style.color = Some("#001f3f".to_owned());
    app.set_char_style(caret(&app, "p1+4"), caret(&app, "p1+7"), &style)
        .expect("styles");

    for form in [Form::Flat, Form::Package] {
        let bytes = app.save_bytes(form).expect("saves");
        let back = App::new();
        back.open_bytes("doc", &bytes).expect("opens");
        assert_eq!(text(&back), text(&app), "{form:?}");
        assert_eq!(
            back.char_style(caret(&back, "p1+4"), caret(&back, "p1+7"))
                .expect("reads"),
            style,
            "{form:?}"
        );
        assert!(
            back.char_style(caret(&back, "p1+0"), caret(&back, "p1+3"))
                .expect("reads")
                .is_plain(),
            "{form:?}: and the plain part stayed plain"
        );
    }
}

/// R6: a formatting edit whose style the file already declares splices like any other, so one
/// bold word is one line of `git diff` rather than a regenerated document.
#[test]
fn reusing_a_style_the_file_already_has_still_splices() {
    // Two paragraphs, the second already bold — so the file declares a bold text style.
    let app = app(&["one two", "three four"]);
    app.set_char_style(caret(&app, "p2+0"), caret(&app, "p2+5"), &bold())
        .expect("styles");
    let bytes = app.save_bytes(Form::Flat).expect("saves");

    let opened = App::new();
    opened.open_bytes("doc.fodt", &bytes).expect("opens");
    opened
        .set_char_style(caret(&opened, "p1+0"), caret(&opened, "p1+3"), &bold())
        .expect("styles");
    let after = opened.save_bytes(Form::Flat).expect("saves");

    let before = String::from_utf8(bytes).expect("utf-8");
    let after = String::from_utf8(after).expect("utf-8");
    let changed = before
        .lines()
        .zip(after.lines())
        .filter(|(a, b)| a != b)
        .count();
    assert_eq!(
        before.lines().count(),
        after.lines().count(),
        "no new lines"
    );
    assert_eq!(changed, 1, "one paragraph edited, one line different");
}

/// The bug a real `grind text image` run found: a document with no image never declared
/// `draw:`/`svg:`, and splicing patches one block element without ever touching the root tag
/// that would have to carry the declaration — so an image inserted into one of those has to
/// force a regenerate, or the spliced file comes back with an undeclared namespace prefix that
/// nothing, including this crate's own reader, can parse as the image it is.
#[test]
fn an_image_that_needs_a_namespace_the_file_never_declared_forces_a_regenerate() {
    let app = app(&["a tiny picture"]);
    let bytes = app.save_bytes(Form::Flat).expect("saves");
    assert!(
        !String::from_utf8_lossy(&bytes).contains("xmlns:draw"),
        "the fixture must not already declare it, or this proves nothing"
    );

    let opened = App::new();
    opened.open_bytes("doc.fodt", &bytes).expect("opens");
    opened
        .insert_image(
            caret(&opened, "p1+0"),
            "image/png".to_owned(),
            vec![1, 2, 3],
            None,
            None,
        )
        .expect("inserts");
    let after = opened.save_bytes(Form::Flat).expect("saves");
    let after = String::from_utf8(after).expect("utf-8");
    assert!(
        after.contains("xmlns:draw") && after.contains("xmlns:svg"),
        "a regenerated document declares what it now uses: {after}"
    );

    // And the result is not just well-formed-looking but actually readable — the same image
    // comes back out.
    let reread = App::new();
    reread
        .open_bytes("doc.fodt", after.as_bytes())
        .expect("the regenerated document parses");
    assert_eq!(
        reread.input_text(0).expect("reads"),
        "\u{fffc}a tiny picture"
    );
}
