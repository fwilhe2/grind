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
