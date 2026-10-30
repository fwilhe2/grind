// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The rules that run on every paragraph, whatever its language (`doc/grammar.md` §4).

mod common;
use common::{found, found_picky, nothing};

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
}

#[test]
fn spaces_that_lay_a_line_out_are_not() {
    nothing("Name\tValue", None);
    nothing("Ends here.  ", None);
    nothing("  Indented.", None);
    nothing("Day       Month       Year", None);
    nothing("Office  \u{2610}", None);
    nothing("6.1  Are instructions displayed?", None);
    nothing("The above named does         does not", None);
}

#[test]
fn two_spaces_after_a_sentence_are_a_style_finding() {
    nothing("End.  Next one.", None);
    assert_eq!(found_picky("End.  Next one.", None), ["double-space:  "]);
}

#[test]
fn doubled_punctuation_is_found_and_an_ellipsis_is_not() {
    assert_eq!(found("Stop!! Now.", None), ["repeated-punctuation:!!"]);
    assert_eq!(found("Red,, green.", None), ["repeated-punctuation:,,"]);
    nothing("Wait... what?!", None);
    nothing("Well…", None);
}

#[test]
fn a_missing_space_after_punctuation_is_found() {
    assert_eq!(
        found("It ended.Then it began.", None),
        ["missing-space-after:."]
    );
    assert_eq!(
        found("Red,green and blue.", None),
        ["missing-space-after:,"]
    );
    assert_eq!(found("Why?Because.", None), ["missing-space-after:?"]);
}

#[test]
fn punctuation_inside_a_number_an_address_or_an_abbreviation_is_not() {
    nothing("It costs 1,50 or 3.14 at 10:30.", None);
    nothing("See e.g. the guide, i.e. this one.", None);
    nothing("Mail ann@example.org or visit www.example.org/help.", None);
    nothing("Open lib.rs and README.md now.", None);
    nothing("Rule z.B. gilt, d.h. immer.", Some("de-DE"));
    nothing("John 3:16 says so.", None);
}

#[test]
fn a_sentence_starting_in_lowercase_is_found() {
    assert_eq!(
        found("It rained. then it stopped.", None),
        ["sentence-start-lowercase:then"]
    );
    assert_eq!(
        found("Really? yes.", None),
        ["sentence-start-lowercase:yes"]
    );
}

#[test]
fn a_lowercase_word_after_an_abbreviation_or_a_number_is_not_a_sentence_start() {
    nothing("Fruit, e.g. apples, etc. and more.", Some("en-US"));
    nothing("Bring tools, i.e. a hammer.", Some("en-US"));
    nothing("Am 3. oktober war es.", Some("de-DE"));
    nothing("Das ist z. B. gut, bzw. besser.", Some("de-DE"));
    nothing("Buy an iPhone. iPads too.", None);
    nothing("Wait... then go.", None);
    nothing("He said, “Go.” then left.", None);
}

/// Opt-in: a bracket is often closed a paragraph or a table cell later, which one paragraph
/// cannot see (`doc/grammar.md` §6).
#[test]
fn an_unpaired_bracket_is_found_when_asked_for() {
    assert_eq!(
        found_picky("See the note (below.", None),
        ["unpaired-bracket:("]
    );
    assert_eq!(found_picky("It works) well.", None), ["unpaired-bracket:)"]);
    nothing("See the note (below.", None);
}

#[test]
fn a_list_label_or_a_smiley_is_not_an_unpaired_bracket() {
    nothing("a) the first point", None);
    nothing("1) the first point", None);
    nothing("Thanks :) see you", None);
    nothing("Paired (as here) and (here).", None);
}

/// What the gov.uk corpus drew that was not wrong (`doc/grammar.md` §6).
#[test]
fn names_symbols_abbreviations_and_labels_from_the_corpus_are_left_alone() {
    nothing("Find it on GOV.UK or Gov.UK today.", Some("en-GB"));
    nothing("Exposure value (LEP,d 85 dB).", Some("en-GB"));
    nothing("Creative industries (inc. fashion design).", Some("en-GB"));
    nothing("Quote the DfE ref. number here.", Some("en-GB"));
    nothing("No more than 5 per cent. of the total.", Some("en-GB"));
    nothing("Is it shared? e.g. by panels.", Some("en-GB"));
    nothing("Points c) one and d) another.", Some("en-GB"));
    nothing("First line\nd) the next", Some("en-GB"));
}

#[test]
fn what_the_corpus_drew_that_was_wrong_is_still_found() {
    assert_eq!(
        found("A Local Governing Body,who is not one.", Some("en-GB")),
        ["missing-space-after:,"]
    );
    assert_eq!(
        found("You need a licence. or explain why.", Some("en-GB")),
        ["sentence-start-lowercase:or"]
    );
    assert_eq!(
        found_picky("Read it (HSE - L26)).", Some("en-GB")),
        ["unpaired-bracket:)"]
    );
}
