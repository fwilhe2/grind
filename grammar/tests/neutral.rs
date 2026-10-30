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
