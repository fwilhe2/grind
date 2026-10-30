// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The rules that run on every paragraph, whatever its language (`doc/grammar.md` §4).

mod common;
use common::{found, nothing};

#[test]
fn a_word_twice_is_found_with_the_space_before_the_second() {
    assert_eq!(found("I saw the the cat.", None), ["repeated-word: the"]);
    assert_eq!(
        found("Wir und und ihr.", Some("de-DE")),
        ["repeated-word: und"]
    );
    assert_eq!(found("The the end.", None), ["repeated-word: the"]);
}

#[test]
fn a_word_twice_that_is_right_twice_is_not() {
    nothing("He said that that was fine.", Some("en-US"));
    nothing("She had had enough.", Some("en-US"));
    nothing("Die Frauen, die die Bücher lasen.", Some("de-DE"));
    nothing("Ich weiß, dass das das Ende ist.", Some("de-DE"));
    nothing("Bora Bora is far.", None);
    nothing("Version 2 2 times.", None);
    nothing("the, the", None);
}

#[test]
fn a_space_before_punctuation_is_found() {
    assert_eq!(found("Yes , please.", None), ["space-before-punctuation: "]);
    assert_eq!(found("The end .", None), ["space-before-punctuation: "]);
    assert_eq!(
        found("Really ?", Some("en-US")),
        ["space-before-punctuation: "]
    );
}

#[test]
fn a_space_before_punctuation_that_belongs_there_is_not() {
    nothing("Vraiment ? Oui ; bien sûr !", Some("fr-FR"));
    nothing("And so on ...", None);
    nothing("It costs .50 a piece.", None);
    nothing("Thanks :)", None);
    nothing("(see below)", None);
}

#[test]
fn two_spaces_between_words_are_found() {
    assert_eq!(found("One  two.", None), ["double-space:  "]);
    assert_eq!(found("End.  Next one.", None), ["double-space:  "]);
}

#[test]
fn spaces_at_a_line_end_or_before_a_tab_are_not() {
    nothing("Name\tValue", None);
    nothing("Ends here.  ", None);
    nothing("  Indented.", None);
}

#[test]
fn doubled_punctuation_is_found_and_an_ellipsis_is_not() {
    assert_eq!(found("Stop!! Now.", None), ["repeated-punctuation:!!"]);
    assert_eq!(found("Red,, green.", None), ["repeated-punctuation:,,"]);
    nothing("Wait... what?!", None);
    nothing("Well…", None);
}
