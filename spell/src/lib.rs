// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The spelling dictionaries (`doc/spelling.md`): Hunspell's, compiled in, checked by
//! [`spellbook`], and handed to a [`grind_text::App`] as a [`Lexicon`].
//!
//! `grind-text` decides what a word is and which words are worth checking; this crate only
//! answers whether one is spelled right. It is a separate crate for the reason `grind-print` is:
//! two megabytes of word lists belong in the binaries that check spelling and in no other.
//!
//! **One language per document**, picked by [`choose`] in a fixed order — the one somebody
//! named, then the one the document states, then the one that knows most of its words — and
//! beside it the person's own word list ([`personal`]), which is theirs rather than the
//! document's and so is never written into one.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock, RwLock};

use grind_text::{App, Lexicon};

/// A language this build has a dictionary for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Language {
    /// American English — SCOWL's `en_US`, size 60, the Hunspell project's own.
    English,
    /// German, reformed spelling — igerman98's `de_DE`.
    German,
}

impl Language {
    /// Every language this build carries, in the order a guess prefers on a tie.
    pub const ALL: [Language; 2] = [Language::English, Language::German];

    /// The BCP 47 tag of the dictionary itself.
    pub fn tag(self) -> &'static str {
        match self {
            Language::English => "en-US",
            Language::German => "de-DE",
        }
    }

    /// Its name, in itself — what a picker shows, since somebody looking for German looks for
    /// *Deutsch*.
    pub fn name(self) -> &'static str {
        match self {
            Language::English => "English (US)",
            Language::German => "Deutsch",
        }
    }

    /// The dictionary for a BCP 47 tag or a POSIX locale name — `de`, `de-AT`, `de_DE.UTF-8`,
    /// `en-GB`. Matched on the language alone: one dictionary per language is what this build
    /// carries, so `en-GB` is checked as American English and says so in every message
    /// (`doc/spelling.md`, "Regional spellings" — a named gap).
    pub fn from_tag(tag: &str) -> Option<Language> {
        let language = tag
            .split(['-', '_', '.', '@'])
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        match language.as_str() {
            "en" => Some(Language::English),
            "de" => Some(Language::German),
            _ => None,
        }
    }

    fn files(self) -> (&'static str, &'static str) {
        match self {
            Language::English => (
                include_str!("../dictionaries/en-US.aff"),
                include_str!("../dictionaries/en-US.dic"),
            ),
            Language::German => (
                include_str!("../dictionaries/de-DE.aff"),
                include_str!("../dictionaries/de-DE.dic"),
            ),
        }
    }

    /// The parsed dictionary — built the first time it is asked for (tens of milliseconds) and
    /// shared for the life of the process.
    pub fn dictionary(self) -> &'static spellbook::Dictionary {
        static CELLS: [OnceLock<spellbook::Dictionary>; 2] = [OnceLock::new(), OnceLock::new()];
        CELLS[self as usize].get_or_init(|| {
            let (aff, dic) = self.files();
            spellbook::Dictionary::new(aff, dic)
                .unwrap_or_else(|e| panic!("the bundled {} dictionary parses: {e}", self.tag()))
        })
    }
}

/// A [`Lexicon`]: one language's dictionary, and the words somebody has said are right.
pub struct Speller {
    language: Language,
    /// The personal list and whatever was added this session — "Add to Dictionary" and
    /// "Ignore" both land here; only the first also writes [`personal`]'s file.
    words: RwLock<BTreeSet<String>>,
}

impl Speller {
    pub fn new(language: Language, words: BTreeSet<String>) -> Self {
        Speller {
            language,
            words: RwLock::new(words),
        }
    }

    pub fn language_of(&self) -> Language {
        self.language
    }

    /// Accept `word` from now on, for as long as this speller lives.
    pub fn accept(&self, word: &str) {
        self.words
            .write()
            .unwrap()
            .insert(grind_text::spell::normalise(word));
    }
}

impl Lexicon for Speller {
    fn knows(&self, word: &str) -> bool {
        if self.language.dictionary().check(word) {
            return true;
        }
        let words = self.words.read().unwrap();
        // A listed word is accepted as listed and capitalised, the way Hunspell treats its own:
        // `grind` listed means `Grind` at the start of a sentence is right too.
        words.contains(word) || words.contains(&lower_first(word))
    }

    fn suggest(&self, word: &str) -> Vec<String> {
        let mut out = Vec::new();
        self.language.dictionary().suggest(word, &mut out);
        out
    }

    fn language(&self) -> &str {
        self.language.tag()
    }
}

fn lower_first(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => first.to_lowercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// Why a document is being checked in the language it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// Somebody named it: `--language`, or a shell's picker.
    Named,
    /// The document states it (`fo:language` on its `Standard` style).
    Stated,
    /// It states none, and this dictionary knew the most of its words.
    Guessed,
    /// It states none and has no words to judge by — the desktop's language, or English.
    Default,
}

impl Source {
    pub fn label(self) -> &'static str {
        match self {
            Source::Named => "chosen",
            Source::Stated => "stated by the document",
            Source::Guessed => "guessed from the text",
            Source::Default => "the default",
        }
    }
}

/// The language a document is checked in, and why.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Choice {
    pub language: Language,
    pub source: Source,
}

/// What [`choose`] answers when there is nothing to check in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Unchecked {
    /// The document states a language this build has no dictionary for. Checking French in
    /// English would underline every word, so it is not checked at all.
    NoDictionary(String),
    /// The document says it is in no language (`zxx`) — code, a list of part numbers.
    NoLanguage,
}

impl std::fmt::Display for Unchecked {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Unchecked::NoDictionary(tag) => write!(
                f,
                "the document is in {tag}, and this build has no dictionary for it"
            ),
            Unchecked::NoLanguage => write!(f, "the document says it is in no language"),
        }
    }
}

/// Which language to check `app`'s document in: `named` if somebody named one, then the one the
/// document states, then a guess from its words, then the desktop's language, then English.
pub fn choose(app: &App, named: Option<Language>) -> Result<Choice, Unchecked> {
    let choice = |language, source| Ok(Choice { language, source });
    if let Some(language) = named {
        return choice(language, Source::Named);
    }
    if let Some(tag) = app.language() {
        if tag.eq_ignore_ascii_case("zxx") {
            return Err(Unchecked::NoLanguage);
        }
        return match Language::from_tag(&tag) {
            Some(language) => choice(language, Source::Stated),
            None => Err(Unchecked::NoDictionary(tag)),
        };
    }
    let spellers: Vec<Speller> = Language::ALL
        .iter()
        .map(|l| Speller::new(*l, BTreeSet::new()))
        .collect();
    let lexicons: Vec<&dyn Lexicon> = spellers.iter().map(|s| s as &dyn Lexicon).collect();
    if let Some(at) = app.guess_language(&lexicons) {
        return choice(Language::ALL[at], Source::Guessed);
    }
    let desktop = grind_core::locale::from_desktop().and_then(|l| Language::from_tag(&l.tag()));
    choice(desktop.unwrap_or(Language::English), Source::Default)
}

/// [`choose`] a language and attach a [`Speller`] for it — with the person's own words
/// ([`personal::load`]) — to `app`. The speller is handed back too, so a shell can
/// [`Speller::accept`] a word into it. With nothing to check in, `app` is left checking nothing.
pub fn attach(app: &App, named: Option<Language>) -> Result<(Choice, Arc<Speller>), Unchecked> {
    match choose(app, named) {
        Ok(choice) => {
            let speller = Arc::new(Speller::new(choice.language, personal::load()));
            app.set_lexicon(Some(speller.clone()));
            Ok((choice, speller))
        }
        Err(why) => {
            app.set_lexicon(None);
            Err(why)
        }
    }
}

/// What somebody has said about spelling for one window — a shell's Spelling menu, `:spell`.
/// The session's rather than the document's: the document's own language is read and never
/// written (`doc/spelling.md`, "Setting the document's language").
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Setting {
    /// [`choose`]'s order: the language the document states, else the one whose dictionary
    /// knows most of its words.
    #[default]
    Automatic,
    In(Language),
    Off,
}

impl Setting {
    /// Every choice, in the order a menu lists them.
    pub const ALL: [Setting; 4] = [
        Setting::Automatic,
        Setting::In(Language::English),
        Setting::In(Language::German),
        Setting::Off,
    ];

    /// `auto`, `off`, or a dictionary's tag — what a menu's state and a typed command carry.
    pub fn tag(self) -> &'static str {
        match self {
            Setting::Automatic => "auto",
            Setting::In(language) => language.tag(),
            Setting::Off => "off",
        }
    }

    /// The other way round, taking any tag [`Language::from_tag`] does — `de` as well as
    /// `de-DE`. `None` for a word that is none of them.
    pub fn from_tag(tag: &str) -> Option<Setting> {
        match tag.trim() {
            "auto" | "automatic" => Some(Setting::Automatic),
            "off" | "none" => Some(Setting::Off),
            tag => Language::from_tag(tag).map(Setting::In),
        }
    }

    /// What a menu calls it.
    pub fn label(self) -> &'static str {
        match self {
            Setting::Automatic => "Automatic",
            Setting::In(language) => language.name(),
            Setting::Off => "Off",
        }
    }

    /// Attach what this setting means to `app`: `Ok(None)` when it is [`Setting::Off`] and
    /// nothing is checked, otherwise [`attach`]'s answer.
    pub fn apply(self, app: &App) -> Result<Option<(Choice, Arc<Speller>)>, Unchecked> {
        match self {
            Setting::Off => {
                app.set_lexicon(None);
                Ok(None)
            }
            Setting::Automatic => attach(app, None).map(Some),
            Setting::In(language) => attach(app, Some(language)).map(Some),
        }
    }
}

/// Whether a window checking in `current` should [`Setting::apply`] again because the document
/// has grown into another language — the first sentences typed into an empty window deciding
/// it rather than the desktop's language. Only while the setting is automatic and the language
/// only a guess, and only below [`grind_text::spell::GUESS_SAMPLE`] words, past which a guess
/// cannot change; `words` is the document's count, which every shell already has for its
/// status bar.
pub fn reguess(app: &App, setting: Setting, current: Choice, words: usize) -> bool {
    let guessing = matches!(current.source, Source::Guessed | Source::Default);
    if setting != Setting::Automatic || !guessing || words > grind_text::spell::GUESS_SAMPLE {
        return false;
    }
    choose(app, None)
        .is_ok_and(|now| now.language != current.language && now.source == Source::Guessed)
}

/// [`attach`] in the document's own language, unless a dictionary is attached already — what a
/// shell with no spelling interface of its own calls before it lints, so its problems pane lists
/// misspellings too. Quietly does nothing when there is nothing to check in.
pub fn ensure(app: &App) {
    if app.spelling_language().is_none() {
        let _ = attach(app, None);
    }
}

/// The person's own word list: one word per line, in `$XDG_CONFIG_HOME/grind/words`
/// (`~/.config/grind/words`, or `%APPDATA%\grind\words` on Windows). Theirs rather than any
/// document's, so one list serves every language and every document, and nothing here writes it
/// into a file it saves.
pub mod personal {
    use super::*;

    /// Where the list lives, or `None` with no home directory to put it in.
    pub fn path() -> Option<PathBuf> {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME")
                    .filter(|v| !v.is_empty())
                    .map(|home| PathBuf::from(home).join(".config"))
            })
            .or_else(|| std::env::var_os("APPDATA").map(PathBuf::from))?;
        Some(base.join("grind").join("words"))
    }

    /// The words in a list's text: one per line, blank lines and `#` comments ignored.
    pub fn parse(text: &str) -> BTreeSet<String> {
        text.lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .map(grind_text::spell::normalise)
            .collect()
    }

    /// The list on disk, or an empty one when there is none.
    pub fn load() -> BTreeSet<String> {
        path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .map_or_else(BTreeSet::new, |text| parse(&text))
    }

    /// Add `word` to the list on disk — appended, so a list somebody keeps by hand keeps its
    /// order and its comments. Nothing is written when the word is already there.
    pub fn add(word: &str) -> std::io::Result<PathBuf> {
        let path = path().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::NotFound, "no home directory")
        })?;
        let word = grind_text::spell::normalise(word.trim());
        let existing = std::fs::read_to_string(&path).unwrap_or_default();
        if parse(&existing).contains(&word) {
            return Ok(path);
        }
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let mut text = existing;
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(&word);
        text.push('\n');
        std::fs::write(&path, text)?;
        Ok(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use grind_text::BlockKind;

    fn english() -> Speller {
        Speller::new(Language::English, BTreeSet::new())
    }

    #[test]
    fn both_dictionaries_parse_and_know_their_own_words() {
        let en = english();
        let de = Speller::new(Language::German, BTreeSet::new());
        assert!(en.knows("receive") && en.knows("Receive") && en.knows("don't"));
        assert!(!en.knows("recieve"));
        assert_eq!(
            en.suggest("recieve").first().map(String::as_str),
            Some("receive")
        );
        // Compounds are German's whole difficulty, and the reason for Hunspell over a word list.
        for word in [
            "Rechtschreibprüfung",
            "Textverarbeitungsprogramm",
            "Straße",
            "Ärger",
        ] {
            assert!(de.knows(word), "{word}");
        }
        assert!(!de.knows("Strasse"));
        assert!(de.suggest("Strasse").contains(&"Straße".to_owned()));
    }

    #[test]
    fn a_tag_names_a_language_whatever_its_region_or_spelling() {
        for tag in ["de", "de-AT", "de_DE.UTF-8", "DE"] {
            assert_eq!(Language::from_tag(tag), Some(Language::German), "{tag}");
        }
        assert_eq!(Language::from_tag("en-GB"), Some(Language::English));
        assert_eq!(Language::from_tag("fr-FR"), None);
        assert_eq!(Language::from_tag(""), None);
    }

    #[test]
    fn a_personal_word_is_known_as_listed_and_capitalised() {
        let speller = Speller::new(Language::English, personal::parse("# mine\ngrind\n\nODF\n"));
        assert!(speller.knows("grind") && speller.knows("Grind"));
        assert!(!speller.knows("grnid"));
        speller.accept("Florian’s");
        assert!(speller.knows("Florian's"));
    }

    #[test]
    fn the_language_is_named_then_stated_then_guessed() {
        let app = App::new();
        app.insert(
            0,
            BlockKind::Paragraph,
            "Das ist ein Haus mit einem roten Dach.",
        )
        .unwrap();
        assert_eq!(
            choose(&app, None),
            Ok(Choice {
                language: Language::German,
                source: Source::Guessed
            })
        );
        assert_eq!(
            choose(&app, Some(Language::English)).unwrap().source,
            Source::Named
        );
        let (choice, _) = attach(&app, None).unwrap();
        assert_eq!(choice.language, Language::German);
        assert_eq!(app.spelling_language().as_deref(), Some("de-DE"));
        app.insert(1, BlockKind::Paragraph, "Ein Fehller.").unwrap();
        let wrong = app.misspellings(0..2);
        assert_eq!(wrong.len(), 1);
        assert_eq!(wrong[0].word, "Fehller");
    }

    #[test]
    fn a_setting_round_trips_through_its_tag_and_off_detaches() {
        for setting in Setting::ALL {
            assert_eq!(Setting::from_tag(setting.tag()), Some(setting));
        }
        for language in Language::ALL {
            assert!(
                Setting::ALL.contains(&Setting::In(language)),
                "{language:?}"
            );
        }
        assert_eq!(Setting::from_tag("de"), Some(Setting::In(Language::German)));
        assert_eq!(Setting::from_tag("klingon"), None);
        let app = App::new();
        assert!(Setting::Automatic.apply(&app).unwrap().is_some());
        assert!(app.spelling_language().is_some());
        assert!(Setting::Off.apply(&app).unwrap().is_none());
        assert_eq!(app.spelling_language(), None);
    }

    #[test]
    fn a_guess_is_reconsidered_as_the_document_grows_and_a_choice_never_is() {
        let app = App::new();
        let (first, _) = attach(&app, Some(Language::English)).unwrap();
        let guessed = Choice {
            source: Source::Default,
            ..first
        };
        app.insert(
            0,
            BlockKind::Paragraph,
            "Das ist ein Haus mit einem roten Dach.",
        )
        .unwrap();
        assert!(reguess(&app, Setting::Automatic, guessed, 8));
        assert!(
            !reguess(&app, Setting::Automatic, first, 8),
            "a named language stays"
        );
        assert!(!reguess(&app, Setting::In(Language::English), guessed, 8));
        assert!(
            !reguess(&app, Setting::Automatic, guessed, 10_000),
            "past the sample"
        );
    }

    #[test]
    fn a_stated_language_with_no_dictionary_is_not_checked_at_all() {
        let fodt = |lang: &str| {
            format!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
<office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" office:version="1.4" office:mimetype="application/vnd.oasis.opendocument.text">
 <office:styles><style:default-style style:family="paragraph"><style:text-properties fo:language="{lang}"/></style:default-style></office:styles>
 <office:body><office:text><text:p>Bonjour le monde</text:p></office:text></office:body>
</office:document>"#
            )
        };
        let app = App::new();
        app.open_bytes("a.fodt", fodt("fr").as_bytes()).unwrap();
        assert_eq!(
            choose(&app, None),
            Err(Unchecked::NoDictionary("fr".into()))
        );
        assert!(attach(&app, None).is_err());
        assert_eq!(app.spelling_language(), None);
        app.open_bytes("a.fodt", fodt("zxx").as_bytes()).unwrap();
        assert_eq!(choose(&app, None), Err(Unchecked::NoLanguage));
        app.open_bytes("a.fodt", fodt("de").as_bytes()).unwrap();
        assert_eq!(choose(&app, None).unwrap().source, Source::Stated);
    }
}
