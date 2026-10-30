// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The English rules (`doc/grammar.md` §4): each finds what it is for, and leaves alone what
//! only looks like it.

mod common;
use common::{found, nothing};

const EN: Option<&str> = Some("en-GB");

#[test]
fn a_or_an_goes_by_the_sound_of_the_next_word() {
    assert_eq!(found("I ate a apple for the first time.", EN), ["a-an:a"]);
    assert_eq!(found("It is an test.", EN), ["a-an:an"]);
    assert_eq!(found("A apple a day is the rule.", EN), ["a-an:A"]);
    assert_eq!(found("Wait an minute for the bus.", EN), ["a-an:an"]);
}

#[test]
fn a_or_an_by_sound_rather_than_by_letter_is_left_alone() {
    nothing("It took an hour, an honest one.", EN);
    nothing("He is an heir to an honour.", EN);
    nothing("A user, a university, a unique one-off.", EN);
    nothing("A European union, a one-way street.", EN);
    nothing("An umbrella and an unusual idea.", EN);
    nothing("Take vitamin A and vitamin E daily.", EN);
    nothing("Option a or option b.", EN);
    nothing("Score a 9 or an 8.", EN);
}

#[test]
fn could_of_is_could_have() {
    assert_eq!(found("You could of asked the man.", EN), ["could-of:of"]);
    assert_eq!(found("They should of known that.", EN), ["could-of:of"]);
    nothing("You could of course ask.", EN);
    nothing("The kind of thing that would of necessity follow.", EN);
}

#[test]
fn a_comparison_takes_than() {
    assert_eq!(found("It is better then nothing.", EN), ["than-then:then"]);
    assert_eq!(
        found("More then ten came to the party.", EN),
        ["than-then:then"]
    );
    nothing("Spend more, then rest.", EN);
}

#[test]
fn its_and_its_apostrophe() {
    assert_eq!(
        found("I think its a good idea for the team.", EN),
        ["its-it's:its"]
    );
    assert_eq!(
        found("The dog wagged it's own tail.", EN),
        ["its-it's:it's"]
    );
    nothing("The firm and its staff.", EN);
    nothing("It's a good idea.", EN);
}

#[test]
fn your_welcome_and_their_is() {
    assert_eq!(
        found("Thanks for the gift! Your welcome.", EN),
        ["your-you're:Your"]
    );
    assert_eq!(found("I know their is a way.", EN), ["there-their:their"]);
    nothing("Your welcome pack is here.", EN);
    nothing("Their island is small.", EN);
}

#[test]
fn a_lone_lowercase_i_is_a_capital() {
    assert_eq!(
        found("Then i went home with the dog.", EN),
        ["lowercase-i:i"]
    );
    nothing("That is, i.e. this one.", EN);
    nothing("See part (i) and i) above.", EN);
}

#[test]
fn english_rules_do_not_run_on_german() {
    nothing("Ich habe a Apfel und an Birne.", Some("de-DE"));
}

/// The corpus's Welsh forms state `en-GB`: a paragraph with no English in it is not English,
/// whatever the document says (`doc/grammar.md` decision 4).
#[test]
fn a_paragraph_in_another_language_is_not_checked_as_the_document_states() {
    nothing("Na – Ewch i gwestiwn 19", EN);
    nothing("A oes angen cymorth arnoch i gyfathrebu a deall?", EN);
    nothing("Hoffech chi i ni chwilio'r gofrestr?", EN);
}

/// Said as letters or as a word, the spelling cannot tell: `a MAPPA meeting`, `an FOI request`.
#[test]
fn an_abbreviation_takes_either_article() {
    nothing("Complete a SSG form or a MAPPA meeting.", EN);
    nothing("Shared through a FOI request.", EN);
}

/// The price of the rule above: a paragraph with none of English's common words is not checked
/// as English, however short and however English it looks.
#[test]
fn a_paragraph_without_a_common_english_word_is_not_checked_as_english() {
    nothing("I ate a apple.", EN);
}

#[test]
fn a_word_said_as_letters_or_a_uni_word_takes_its_sound() {
    nothing("It is SPEC with an rng line in the file.", EN);
    nothing("It is a Unicode algorithm and a unicycle.", EN);
    nothing("It is an unimportant and an uninformed view.", EN);
}

#[test]
fn a_particle_before_its_preposition_is_not_a_repeated_word() {
    nothing("It is how the palette was looked at at all.", EN);
}
