// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The spelling dictionaries (`doc/spelling.md`): Hunspell's, compiled in, checked by
//! [`spellbook`], and handed to either application's `App` as a [`Lexicon`] through
//! [`grind_core::spell::Spelled`] — this crate names neither application. `grind-text` decides
//! what a word is and which are checked; this crate answers whether one is spelled right. Separate
//! so two megabytes of word lists go only into binaries that check spelling.
//!
//! **Every language, unless somebody asks for one.** A word is right when any dictionary knows it,
//! so a German letter quoting English underlines neither. The document's own language —
//! [`choose`]: the one named, else the one the document states, else the one knowing most of its
//! words — is asked first and gives suggestions; naming one ([`Setting::In`]) checks in it alone.
//! The person's own word list ([`personal`]) is theirs, never written into a document.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock, RwLock};

use grind_core::spell::{GUESS_SAMPLE, Lexicon, Spelled, normalise};

/// A language this build has a dictionary for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Language {
    /// American English — SCOWL's `en_US`, size 60, the Hunspell project's own.
    English,
    /// German, reformed spelling — igerman98's `de_DE`.
    German,
    /// French — Grammalecte's, the reformed spelling of 1990 and its older variants both.
    French,
    /// Spanish — RLA-ES's `es_ES`, the Spanish of Spain.
    Spanish,
    /// Italian — the Italian Writing Aids' `it_IT`.
    Italian,
    /// European Portuguese — the University of Minho's `pt_PT`, in the 1990 agreement's spelling.
    Portuguese,
    /// Polish — the Polish Native Lang Project's `pl_PL`.
    Polish,
}

impl Language {
    /// Every language this build carries, in the order a guess prefers on a tie.
    pub const ALL: [Language; 7] = [
        Language::English,
        Language::German,
        Language::French,
        Language::Spanish,
        Language::Italian,
        Language::Portuguese,
        Language::Polish,
    ];

    /// The BCP 47 tag of the dictionary itself.
    pub fn tag(self) -> &'static str {
        match self {
            Language::English => "en-US",
            Language::German => "de-DE",
            Language::French => "fr-FR",
            Language::Spanish => "es-ES",
            Language::Italian => "it-IT",
            Language::Portuguese => "pt-PT",
            Language::Polish => "pl-PL",
        }
    }

    /// Its name, in itself — what a picker shows, since somebody looking for German looks for
    /// *Deutsch*.
    pub fn name(self) -> &'static str {
        match self {
            Language::English => "English (US)",
            Language::German => "Deutsch",
            Language::French => "Français",
            Language::Spanish => "Español",
            Language::Italian => "Italiano",
            Language::Portuguese => "Português",
            Language::Polish => "Polski",
        }
    }

    /// The flag of the country whose spelling the dictionary is — `🇩🇪` for `de-DE` — as the two
    /// regional-indicator symbols an emoji font draws as one flag. A dictionary *is* one
    /// country's spelling (`pt-PT`, not Brazil's), which is what makes a flag accurate here
    /// rather than the usual mistake of a flag for a language. Where the platform has no flag
    /// glyphs — Windows draws `DE` for them, GDI no colour glyph at all, a terminal disagrees
    /// with itself about the width — a shell shows [`Language::code`] instead.
    pub fn flag(self) -> &'static str {
        match self {
            Language::English => "\u{1F1FA}\u{1F1F8}",
            Language::German => "\u{1F1E9}\u{1F1EA}",
            Language::French => "\u{1F1EB}\u{1F1F7}",
            Language::Spanish => "\u{1F1EA}\u{1F1F8}",
            Language::Italian => "\u{1F1EE}\u{1F1F9}",
            Language::Portuguese => "\u{1F1F5}\u{1F1F9}",
            Language::Polish => "\u{1F1F5}\u{1F1F1}",
        }
    }

    /// The language in two capitals — `DE` — where a flag cannot be drawn.
    pub fn code(self) -> &'static str {
        match self {
            Language::English => "EN",
            Language::German => "DE",
            Language::French => "FR",
            Language::Spanish => "ES",
            Language::Italian => "IT",
            Language::Portuguese => "PT",
            Language::Polish => "PL",
        }
    }

    /// The dictionary for a BCP 47 tag or a POSIX locale name — `de`, `de-AT`, `de_DE.UTF-8`,
    /// `en-GB`. Matched on the language alone, since there is one dictionary per language: `en-GB`
    /// is checked as American English and `pt-BR` as European Portuguese, and each says so
    /// (`doc/spelling.md`, "Regional spellings").
    pub fn from_tag(tag: &str) -> Option<Language> {
        let language = tag
            .split(['-', '_', '.', '@'])
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        match language.as_str() {
            "en" => Some(Language::English),
            "de" => Some(Language::German),
            "fr" => Some(Language::French),
            "es" => Some(Language::Spanish),
            "it" => Some(Language::Italian),
            "pt" => Some(Language::Portuguese),
            "pl" => Some(Language::Polish),
            _ => None,
        }
    }

    #[cfg(feature = "bundled")]
    fn files(self) -> (&'static str, &'static str) {
        macro_rules! pair {
            ($tag:literal) => {
                (
                    include_str!(concat!("../dictionaries/", $tag, ".aff")),
                    include_str!(concat!("../dictionaries/", $tag, ".dic")),
                )
            };
        }
        match self {
            Language::English => pair!("en-US"),
            Language::German => pair!("de-DE"),
            Language::French => pair!("fr-FR"),
            Language::Spanish => pair!("es-ES"),
            Language::Italian => pair!("it-IT"),
            Language::Portuguese => pair!("pt-PT"),
            Language::Polish => pair!("pl-PL"),
        }
    }

    fn cell(self) -> &'static OnceLock<spellbook::Dictionary> {
        static CELLS: [OnceLock<spellbook::Dictionary>; Language::ALL.len()] =
            [const { OnceLock::new() }; Language::ALL.len()];
        &CELLS[self as usize]
    }

    /// Whether this language's dictionary is here: always, in a build that carries them, and
    /// once [`Language::provide`] has been handed it in one that does not.
    pub fn available(self) -> bool {
        cfg!(feature = "bundled") || self.cell().get().is_some()
    }

    /// Hand over the Hunspell files of a dictionary this build does not carry — what the
    /// browser fetches the first time a language is asked for. The first call for a language
    /// wins; the files are parsed here, which is 40 to 300 milliseconds.
    pub fn provide(self, aff: &str, dic: &str) -> Result<(), String> {
        if self.cell().get().is_some() {
            return Ok(());
        }
        let parsed = spellbook::Dictionary::new(aff, dic)
            .map_err(|e| format!("the {} dictionary does not parse: {e}", self.tag()))?;
        let _ = self.cell().set(parsed);
        Ok(())
    }

    /// The parsed dictionary — built the first time it is asked for (40 to 300 milliseconds,
    /// Polish the slowest) and shared for the life of the process. In a build without the
    /// bundled dictionaries it is the one [`Language::provide`] was given, and asking for a
    /// language that was never provided is a bug in the caller: ask [`Language::available`].
    pub fn dictionary(self) -> &'static spellbook::Dictionary {
        self.cell().get_or_init(|| {
            #[cfg(feature = "bundled")]
            {
                let (aff, dic) = self.files();
                spellbook::Dictionary::new(aff, dic)
                    .unwrap_or_else(|e| panic!("the bundled {} dictionary parses: {e}", self.tag()))
            }
            #[cfg(not(feature = "bundled"))]
            panic!("the {} dictionary was never provided", self.tag())
        })
    }
}

/// A [`Lexicon`]: one language's dictionary — or that one first and every other after it — and
/// the words somebody has said are right.
pub struct Speller {
    language: Language,
    /// Whether a word any other dictionary knows is right too ([`Speller::every`]).
    every: bool,
    /// The personal list and whatever was added this session — "Add to Dictionary" and
    /// "Ignore" both land here; only the first also writes [`personal`]'s file.
    words: RwLock<BTreeSet<String>>,
}

impl Speller {
    /// A speller for `language` alone.
    pub fn new(language: Language, words: BTreeSet<String>) -> Self {
        Speller {
            language,
            every: false,
            words: RwLock::new(words),
        }
    }

    /// A speller that asks `first` and then every other language this build has — what a document
    /// is checked with unless somebody named one. Suggestions, and the language messages name,
    /// are `first`'s. Where dictionaries are fetched rather than carried, "every other" is the
    /// ones that have arrived.
    pub fn every(first: Language, words: BTreeSet<String>) -> Self {
        Speller {
            every: true,
            ..Speller::new(first, words)
        }
    }

    /// Whether a word any dictionary knows is accepted, rather than only [`Speller::language_of`]'s.
    pub fn is_every(&self) -> bool {
        self.every
    }

    pub fn language_of(&self) -> Language {
        self.language
    }

    /// Accept `word` from now on, for as long as this speller lives.
    pub fn accept(&self, word: &str) {
        self.words.write().unwrap().insert(normalise(word));
    }

    /// Stop accepting `word` — what removing it from the personal list means for a document
    /// already open. A word the dictionary itself knows stays known, of course.
    pub fn forget(&self, word: &str) {
        self.words.write().unwrap().remove(&normalise(word));
    }
}

impl Lexicon for Speller {
    fn knows(&self, word: &str) -> bool {
        if self.language.dictionary().check(word) {
            return true;
        }
        {
            let words = self.words.read().unwrap();
            // A listed word is accepted as listed and capitalised, the way Hunspell treats its
            // own: `grind` listed means `Grind` at the start of a sentence is right too.
            if words.contains(word) || words.contains(&lower_first(word)) {
                return true;
            }
        }
        // Only a word the first dictionary does not know gets this far, so the others cost
        // nothing for a word spelled right in the document's own language.
        self.every
            && Language::ALL
                .iter()
                .filter(|other| **other != self.language && other.available())
                .any(|other| other.dictionary().check(word))
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
///
/// Unless it was [`Source::Named`], `language` is only the *first* dictionary asked and the one
/// suggestions come from: a word any other dictionary knows is accepted as well ([`Choice::every`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Choice {
    pub language: Language,
    pub source: Source,
}

impl Choice {
    /// Whether every language's words are accepted — anything but a language somebody named.
    pub fn every(self) -> bool {
        self.source != Source::Named
    }

    /// What a status bar shows: `All languages`, or the one language's own name.
    pub fn name(self) -> &'static str {
        match self.every() {
            true => Setting::Automatic.label(),
            false => self.language.name(),
        }
    }

    /// The short form, for a narrow status line: `all`, or the one language's tag.
    pub fn tag(self) -> &'static str {
        match self.every() {
            true => "all",
            false => self.language.tag(),
        }
    }

    /// One sentence on what is checked and why — `every language, suggestions in Deutsch (guessed
    /// from the text)`, or `Deutsch (chosen)`.
    pub fn describe(self) -> String {
        match self.every() {
            true => format!(
                "every language, suggestions in {} ({})",
                self.language.name(),
                self.source.label()
            ),
            false => format!("{} ({})", self.language.name(), self.source.label()),
        }
    }
}

/// What [`choose`] answers when there is nothing to check in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Unchecked {
    /// The document states a language this build has no dictionary for. Checking Dutch in
    /// English would underline every word, so it is not checked at all.
    NoDictionary(String),
    /// The document says it is in no language (`zxx`) — code, a list of part numbers.
    NoLanguage,
    /// This build fetches its dictionaries and this one has not arrived yet: whoever asks
    /// fetches it, [`Language::provide`]s it, and asks again.
    NotLoaded(Language),
}

impl std::fmt::Display for Unchecked {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Unchecked::NoDictionary(tag) => write!(
                f,
                "the document is in {tag}, and this build has no dictionary for it"
            ),
            Unchecked::NoLanguage => write!(f, "the document says it is in no language"),
            Unchecked::NotLoaded(language) => {
                write!(
                    f,
                    "the {} dictionary has not been loaded yet",
                    language.name()
                )
            }
        }
    }
}

/// Which language to check `app`'s document in: `named` if somebody named one, then the one the
/// document states, then a guess from its words, then the desktop's language, then English.
/// Anything but `named` is the first of every language rather than the only one
/// ([`Choice::every`]).
pub fn choose(app: &(impl Spelled + ?Sized), named: Option<Language>) -> Result<Choice, Unchecked> {
    let choice = |language, source| Ok(Choice { language, source });
    if let Some(language) = named {
        return choice(language, Source::Named);
    }
    if let Some(tag) = app.stated_language() {
        if tag.eq_ignore_ascii_case("zxx") {
            return Err(Unchecked::NoLanguage);
        }
        return match Language::from_tag(&tag) {
            Some(language) => choice(language, Source::Stated),
            None => Err(Unchecked::NoDictionary(tag)),
        };
    }
    let desktop = grind_core::locale::from_desktop().and_then(|l| Language::from_tag(&l.tag()));
    if let Some(language) = guess(app, desktop) {
        return choice(language, Source::Guessed);
    }
    choice(desktop.unwrap_or(Language::English), Source::Default)
}

/// The guess, in two stages so that guessing never costs seven dictionaries.
///
/// First by **common words** ([`Common`]): whichever language's most frequent short words occur
/// most often. It loads nothing, and it tells apart the languages a full dictionary is worst at
/// telling apart — Spanish, Portuguese and Italian share a great many words and very few of
/// their articles and particles. Only a text with not one of them in it (a heading, a list of
/// nouns) falls to the second stage: a vote by the full dictionaries of the desktop's language
/// and English, the two the default would pick between anyway — so at most two are loaded.
fn guess(app: &(impl Spelled + ?Sized), desktop: Option<Language>) -> Option<Language> {
    let sample = app.sample();
    let common: Vec<Common> = Language::ALL.iter().map(|l| Common(*l)).collect();
    let lexicons: Vec<&dyn Lexicon> = common.iter().map(|c| c as &dyn Lexicon).collect();
    if let Some(at) = grind_core::spell::guess(&sample, &lexicons) {
        return Some(Language::ALL[at]);
    }
    let mut fallback = vec![desktop.unwrap_or(Language::English), Language::English];
    fallback.dedup();
    // Only dictionaries that are here: a build that fetches them does not fetch two to guess.
    fallback.retain(|language| language.available());
    if fallback.is_empty() {
        return None;
    }
    let spellers: Vec<Speller> = fallback
        .iter()
        .map(|l| Speller::new(*l, BTreeSet::new()))
        .collect();
    let lexicons: Vec<&dyn Lexicon> = spellers.iter().map(|s| s as &dyn Lexicon).collect();
    grind_core::spell::guess(&sample, &lexicons).map(|at| fallback[at])
}

/// A [`Lexicon`] that knows only a language's commonest short words — articles, pronouns,
/// particles, the forms of *to be* — chosen to be that language's alone where they can be: `la`
/// is French, Spanish and Italian at once and is in none of the lists, `die` and `und` are only
/// German. None has a single letter, because a one-letter word is never a candidate
/// (`doc/spelling.md`, decision 3). What [`guess`] votes with before it loads anything.
struct Common(Language);

impl Common {
    fn words(&self) -> &'static [&'static str] {
        match self.0 {
            Language::English => &[
                "the", "and", "of", "is", "that", "it", "for", "with", "be", "at", "by", "this",
                "have", "from", "or", "are", "not", "but", "which", "you", "we", "they", "he",
                "she", "his", "her", "would", "there", "their", "been", "has", "were", "can",
                "what", "if", "my", "these", "about", "should", "who",
            ],
            Language::German => &[
                "der", "die", "das", "und", "ist", "nicht", "ein", "eine", "einen", "dem", "den",
                "zu", "mit", "sich", "auf", "für", "von", "auch", "im", "sie", "wir", "ich", "er",
                "sind", "war", "wird", "werden", "aber", "oder", "wenn", "noch", "nach", "bei",
                "aus", "wie", "dass", "nur", "haben",
            ],
            Language::French => &[
                "les", "des", "du", "est", "et", "une", "dans", "pour", "pas", "qui", "sur", "au",
                "aux", "avec", "ce", "cette", "elle", "nous", "vous", "ils", "elles", "plus",
                "par", "je", "été", "être", "très", "leur", "sont", "mon", "avez", "avons",
                "aussi", "comme", "tout", "fait", "était", "ces", "peut", "quand",
            ],
            Language::Spanish => &[
                "el", "los", "las", "es", "en", "está", "están", "pero", "más", "muy", "también",
                "cuando", "hay", "eso", "esto", "fue", "yo", "él", "ella", "ellos", "ellas",
                "hasta", "sin", "usted", "porque", "qué", "cómo", "tiene", "tienen", "puede",
                "hacer", "todo", "bien", "aquí", "ahora", "mucho", "estos", "nosotros", "pues",
                "donde",
            ],
            Language::Italian => &[
                "il", "gli", "della", "delle", "dei", "degli", "che", "non", "per", "sono",
                "anche", "più", "questo", "questa", "nel", "nella", "alla", "alle", "sul", "io",
                "lui", "lei", "noi", "voi", "loro", "essere", "stato", "molto", "perché", "hanno",
                "ho", "abbiamo", "anni", "fare", "così", "dove", "tutto", "sempre", "allora",
                "questi",
            ],
            Language::Portuguese => &[
                "os", "um", "uma", "não", "em", "com", "isso", "isto", "também", "muito", "ele",
                "ela", "eles", "elas", "eu", "você", "foi", "são", "mas", "seu", "sua", "pelo",
                "pela", "ao", "aos", "já", "só", "então", "há", "nós", "ainda", "tem", "têm",
                "fazer", "aqui", "agora", "onde", "pode", "muitos", "mesmo",
            ],
            Language::Polish => &[
                "nie", "się", "jest", "że", "od", "za", "jak", "ale", "czy", "tak", "już", "jego",
                "jej", "był", "była", "było", "są", "przez", "przy", "dla", "tylko", "bardzo",
                "może", "jeszcze", "gdy", "które", "który", "która", "oraz", "także", "tym",
                "tego", "jestem", "też", "być", "mnie", "kiedy", "więc", "jako",
            ],
        }
    }
}

impl Lexicon for Common {
    fn knows(&self, word: &str) -> bool {
        let word = word.to_lowercase();
        self.words().contains(&word.as_str())
    }

    fn suggest(&self, _: &str) -> Vec<String> {
        Vec::new()
    }

    fn language(&self) -> &str {
        self.0.tag()
    }
}

/// [`choose`] a language and attach a [`Speller`] for it — with the person's own words
/// ([`personal::load`]) — to `app`: that language alone when it was `named`, otherwise that one
/// first and [`Speller::every`] other after it. The speller is handed back too, so a shell can
/// [`Speller::accept`] a word into it. With nothing to check in, `app` is left checking nothing.
pub fn attach(
    app: &(impl Spelled + ?Sized),
    named: Option<Language>,
) -> Result<(Choice, Arc<Speller>), Unchecked> {
    match choose(app, named) {
        Ok(choice) if !choice.language.available() => {
            app.set_lexicon(None);
            Err(Unchecked::NotLoaded(choice.language))
        }
        Ok(choice) => {
            let speller = Arc::new(match choice.every() {
                true => {
                    warm();
                    Speller::every(choice.language, personal::load())
                }
                false => Speller::new(choice.language, personal::load()),
            });
            app.set_lexicon(Some(speller.clone()));
            Ok((choice, speller))
        }
        Err(why) => {
            app.set_lexicon(None);
            Err(why)
        }
    }
}

/// Parse every dictionary on a thread of its own, once a process — about a quarter of a second
/// and 57 MB in all, Polish half of both — so the first word the document's own language does
/// not know does not wait for six more to load. A build with no threads (the browser) parses a
/// dictionary as it arrives instead.
fn warm() {
    #[cfg(not(target_family = "wasm"))]
    {
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| {
            let _ = std::thread::Builder::new()
                .name("grind-spell-warm".into())
                .spawn(|| {
                    for language in Language::ALL.into_iter().filter(|l| l.available()) {
                        language.dictionary();
                    }
                });
        });
    }
}

/// What somebody has said about spelling for one window — a shell's Spelling menu, `:spell`.
/// The session's rather than the document's: the document's own language is read and never
/// written (`doc/spelling.md`, "Setting the document's language").
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Setting {
    /// A word any dictionary knows is right, with [`choose`]'s language — the one the document
    /// states, else the one whose dictionary knows most of its words — asked first and
    /// suggesting. The default: checking in one language alone has to be asked for.
    #[default]
    Automatic,
    /// That language alone.
    In(Language),
    Off,
}

impl Setting {
    /// Every choice, in the order a menu lists them.
    pub const ALL: [Setting; Language::ALL.len() + 2] = {
        let mut all = [Setting::Automatic; Language::ALL.len() + 2];
        let mut at = 0;
        while at < Language::ALL.len() {
            all[at + 1] = Setting::In(Language::ALL[at]);
            at += 1;
        }
        all[Language::ALL.len() + 1] = Setting::Off;
        all
    };

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
            "auto" | "automatic" | "all" | "any" => Some(Setting::Automatic),
            "off" | "none" => Some(Setting::Off),
            tag => Language::from_tag(tag).map(Setting::In),
        }
    }

    /// Its picture, for a status-bar picker: `🌐` for every language, the dictionary's flag for
    /// one ([`Language::flag`]), and `None` for off, which is a word rather than a picture.
    pub fn flag(self) -> Option<&'static str> {
        match self {
            Setting::Automatic => Some("\u{1F310}"),
            Setting::In(language) => Some(language.flag()),
            Setting::Off => None,
        }
    }

    /// The same in letters, where the platform cannot draw a flag: `ALL`, `DE`, `OFF`.
    pub fn code(self) -> &'static str {
        match self {
            Setting::Automatic => "ALL",
            Setting::In(language) => language.code(),
            Setting::Off => "OFF",
        }
    }

    /// What a picker's button shows: the flag with emoji, the code without — `Off` either way.
    pub fn chip(self, emoji: bool) -> &'static str {
        match (emoji, self.flag()) {
            (true, Some(flag)) => flag,
            (_, None) => "Off",
            (false, Some(_)) => self.code(),
        }
    }

    /// One row of a picker: `🇩🇪  Deutsch`, or `DE  Deutsch`, or plain `Off`.
    pub fn row(self, emoji: bool) -> String {
        match self {
            Setting::Off => self.label().to_owned(),
            _ => format!("{}  {}", self.chip(emoji), self.label()),
        }
    }

    /// What a menu calls it.
    pub fn label(self) -> &'static str {
        match self {
            Setting::Automatic => "All languages",
            Setting::In(language) => language.name(),
            Setting::Off => "Off",
        }
    }

    /// Attach what this setting means to `app`: `Ok(None)` when it is [`Setting::Off`] and
    /// nothing is checked, otherwise [`attach`]'s answer.
    pub fn apply(
        self,
        app: &(impl Spelled + ?Sized),
    ) -> Result<Option<(Choice, Arc<Speller>)>, Unchecked> {
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
/// only a guess, and only below [`GUESS_SAMPLE`] words, past which a guess
/// cannot change; `words` is the document's count, which every shell already has for its
/// status bar.
pub fn reguess(
    app: &(impl Spelled + ?Sized),
    setting: Setting,
    current: Choice,
    words: usize,
) -> bool {
    let guessing = matches!(current.source, Source::Guessed | Source::Default);
    if setting != Setting::Automatic || !guessing || words > GUESS_SAMPLE {
        return false;
    }
    choose(app, None)
        .is_ok_and(|now| now.language != current.language && now.source == Source::Guessed)
}

/// [`attach`] in the document's own language, unless a dictionary is attached already — what a
/// shell with no spelling interface of its own calls before it lints, so its problems pane lists
/// misspellings too. Quietly does nothing when there is nothing to check in.
pub fn ensure(app: &(impl Spelled + ?Sized)) {
    if app.spelling_language().is_none() {
        let _ = attach(app, None);
    }
}

/// `$XDG_CONFIG_HOME/grind` (`~/.config/grind`, or `%APPDATA%\grind` on Windows) — where the
/// person's own spelling files live, or `None` with no home directory to put them in.
fn config_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .filter(|v| !v.is_empty())
                .map(|home| PathBuf::from(home).join(".config"))
        })
        .or_else(|| std::env::var_os("APPDATA").map(PathBuf::from))?;
    Some(base.join("grind"))
}

/// Whether spelling is checked at all, and in what — **remembered**, one [`Setting`] per kind of
/// document, in `$XDG_CONFIG_HOME/grind/spelling`. What makes spelling easy to turn off and
/// keep off: a spreadsheet full of part numbers, names and codes is somewhere a person may not
/// want it, and one choice in any window should hold for the next window too.
///
/// The file is a line per kind — `spreadsheet off`, `text de-DE` — with `#` comments, and a kind
/// it does not name is [`Setting::Automatic`]: checked, in the language [`choose`] picks. Like
/// the word list, it is the person's and never the document's (`doc/spelling.md`, decision 5).
pub mod preference {
    use super::*;
    use grind_core::DocumentKind;

    /// Where the setting lives, or `None` with no home directory to put it in.
    pub fn path() -> Option<PathBuf> {
        Some(config_dir()?.join("spelling"))
    }

    fn key(kind: DocumentKind) -> &'static str {
        match kind {
            DocumentKind::Spreadsheet => "spreadsheet",
            DocumentKind::Text => "text",
            DocumentKind::Presentation => "presentation",
        }
    }

    /// The setting for `kind` in a file's text: the last line naming it wins, and a line this
    /// build cannot read is ignored rather than read as something else.
    pub fn parse(text: &str, kind: DocumentKind) -> Setting {
        text.lines()
            .rev()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .filter_map(|line| line.split_once(char::is_whitespace))
            .filter(|(name, _)| *name == key(kind))
            .find_map(|(_, value)| Setting::from_tag(value))
            .unwrap_or_default()
    }

    /// The remembered setting for `kind` — [`Setting::Automatic`] when nothing was ever chosen.
    pub fn load(kind: DocumentKind) -> Setting {
        path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .map_or_else(Setting::default, |text| parse(&text, kind))
    }

    /// `text` with `kind`'s line replaced by `setting` — or removed, for [`Setting::Automatic`],
    /// so the file only ever holds what somebody chose. Every other line, comments included, is
    /// kept as it was.
    pub fn with(text: &str, kind: DocumentKind, setting: Setting) -> String {
        let mut lines: Vec<String> = text
            .lines()
            .filter(|line| {
                line.trim()
                    .split_once(char::is_whitespace)
                    .is_none_or(|(name, _)| name != key(kind))
            })
            .map(str::to_owned)
            .collect();
        if setting != Setting::Automatic {
            lines.push(format!("{} {}", key(kind), setting.tag()));
        }
        let mut out = lines.join("\n");
        if !out.is_empty() {
            out.push('\n');
        }
        out
    }

    /// Remember `setting` for `kind`. Nothing is written when it is already what is remembered.
    pub fn save(kind: DocumentKind, setting: Setting) -> std::io::Result<PathBuf> {
        let path = path().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::NotFound, "no home directory")
        })?;
        let existing = std::fs::read_to_string(&path).unwrap_or_default();
        if parse(&existing, kind) == setting {
            return Ok(path);
        }
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&path, with(&existing, kind, setting))?;
        Ok(path)
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
        Some(config_dir()?.join("words"))
    }

    /// The words in a list's text: one per line, blank lines and `#` comments ignored.
    pub fn parse(text: &str) -> BTreeSet<String> {
        text.lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .map(normalise)
            .collect()
    }

    /// The list on disk, or an empty one when there is none.
    pub fn load() -> BTreeSet<String> {
        path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .map_or_else(BTreeSet::new, |text| parse(&text))
    }

    /// What somebody typed as a word to add, made into one — trimmed, `’` folded to `'` — or
    /// why it is not one: nothing, or several words, since one line of the list is one word.
    /// Every client's *Add* goes through this, so each refuses the same things.
    pub fn word(typed: &str) -> Result<String, &'static str> {
        let word = normalise(typed.trim());
        match () {
            _ if word.is_empty() => Err("type a word to add"),
            _ if word.chars().any(char::is_whitespace) => Err("one word at a time"),
            _ if word.starts_with('#') => Err("a word cannot begin with #, which marks a comment"),
            _ => Ok(word),
        }
    }

    /// Every word on the list, sorted — what a *Personal Dictionary* view shows.
    pub fn words() -> Vec<String> {
        load().into_iter().collect()
    }

    /// `text` without the line holding `word`; every other line, comments included, as it was.
    pub fn without(text: &str, word: &str) -> String {
        let word = normalise(word.trim());
        let mut out: String = text
            .lines()
            .filter(|line| normalise(line.trim()) != word)
            .collect::<Vec<_>>()
            .join("\n");
        if !out.is_empty() {
            out.push('\n');
        }
        out
    }

    /// Take `word` off the list on disk. `Ok(false)` when it was not there, and nothing is
    /// written then.
    pub fn remove(word: &str) -> std::io::Result<bool> {
        let Some(path) = path() else {
            return Ok(false);
        };
        let Ok(existing) = std::fs::read_to_string(&path) else {
            return Ok(false);
        };
        if !parse(&existing).contains(&normalise(word.trim())) {
            return Ok(false);
        }
        std::fs::write(&path, without(&existing, word))?;
        Ok(true)
    }

    /// Add `word` to the list on disk — appended, so a list somebody keeps by hand keeps its
    /// order and its comments. Nothing is written when the word is already there.
    pub fn add(word: &str) -> std::io::Result<PathBuf> {
        let path = path().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::NotFound, "no home directory")
        })?;
        let word = normalise(word.trim());
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
    use grind_text::{App, BlockKind};

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
    fn every_dictionary_knows_its_own_words_and_corrects_a_slip() {
        // The words each language's checker most needs to get right: its diacritics, and for
        // French and Italian an elision joined across the apostrophe, which `’` must reach as `'`.
        let cases: [(Language, &[&str], &str, &str); 7] = [
            (
                Language::English,
                &["receive", "don't"],
                "recieve",
                "receive",
            ),
            (Language::German, &["Straße", "Ärger"], "Strasse", "Straße"),
            (
                Language::French,
                &["aujourd'hui", "l'école", "Français", "être"],
                "bonjjour",
                "bonjour",
            ),
            (
                Language::Spanish,
                &["mañana", "corazón", "España", "hablamos"],
                "holaa",
                "hola",
            ),
            (
                Language::Italian,
                &["perché", "città", "l'amico", "scrivere"],
                "ciaoo",
                "ciao",
            ),
            (
                Language::Portuguese,
                &["ação", "coração", "português", "você"],
                "voce",
                "você",
            ),
            (
                Language::Polish,
                &["cześć", "źdźbło", "gęś", "przyjaciółmi"],
                "czesc",
                "cześć",
            ),
        ];
        assert_eq!(cases.len(), Language::ALL.len());
        for (language, right, wrong, meant) in cases {
            let speller = Speller::new(language, BTreeSet::new());
            for word in right {
                assert!(speller.knows(word), "{language:?}: {word}");
            }
            assert!(!speller.knows(wrong), "{language:?}: {wrong}");
            assert!(
                speller.suggest(wrong).iter().any(|s| s == meant),
                "{language:?}: {wrong} → {meant}"
            );
        }
    }

    #[test]
    fn each_language_is_guessed_from_a_sentence_of_it() {
        let sentences = [
            (
                Language::English,
                "The house is at the end of the road and they have been there for years.",
            ),
            (
                Language::German,
                "Das Haus steht am Ende der Straße, und wir wohnen dort seit vielen Jahren.",
            ),
            (
                Language::French,
                "La maison est au bout de la rue et nous y vivons depuis des années avec eux.",
            ),
            (
                Language::Spanish,
                "La casa está al final de la calle y vivimos allí desde hace muchos años con ellos.",
            ),
            (
                Language::Italian,
                "La casa è alla fine della strada e ci abitiamo da molti anni con loro.",
            ),
            (
                Language::Portuguese,
                "A casa fica no fim da rua e nós moramos lá há muitos anos com eles.",
            ),
            (
                Language::Polish,
                "Dom jest na końcu ulicy i mieszkamy tam od wielu lat, ale nie jesteśmy sami.",
            ),
        ];
        assert_eq!(sentences.len(), Language::ALL.len());
        for (language, sentence) in sentences {
            let app = App::new();
            app.insert(0, BlockKind::Paragraph, sentence).unwrap();
            assert_eq!(
                choose(&app, None),
                Ok(Choice {
                    language,
                    source: Source::Guessed
                }),
                "{sentence}"
            );
        }
    }

    #[test]
    fn no_common_word_votes_for_two_languages_or_could_never_be_counted() {
        let mut seen = std::collections::BTreeMap::new();
        for language in Language::ALL {
            for word in Common(language).words() {
                assert!(word.chars().count() > 1, "{word} is never a candidate");
                assert_eq!(*word, word.to_lowercase(), "{word}");
                if let Some(other) = seen.insert(*word, language) {
                    panic!("{word} is in both {other:?} and {language:?}");
                }
            }
        }
    }

    #[test]
    fn a_text_with_no_common_word_is_guessed_by_the_dictionaries_it_may_load() {
        let app = App::new();
        app.insert(0, BlockKind::Paragraph, "Quarterly revenue forecast")
            .unwrap();
        assert_eq!(guess(&app, None), Some(Language::English));
        let app = App::new();
        app.insert(
            0,
            BlockKind::Paragraph,
            "Rechtschreibprüfung Textverarbeitung",
        )
        .unwrap();
        assert_eq!(guess(&app, Some(Language::German)), Some(Language::German));
        // French is not loaded for a German desktop, so French nouns alone are not guessed.
        let app = App::new();
        app.insert(0, BlockKind::Paragraph, "Bibliothèque municipale")
            .unwrap();
        assert_eq!(guess(&app, Some(Language::German)), None);
    }

    #[test]
    fn a_tag_names_a_language_whatever_its_region_or_spelling() {
        for tag in ["de", "de-AT", "de_DE.UTF-8", "DE"] {
            assert_eq!(Language::from_tag(tag), Some(Language::German), "{tag}");
        }
        assert_eq!(Language::from_tag("en-GB"), Some(Language::English));
        assert_eq!(Language::from_tag("pt-BR"), Some(Language::Portuguese));
        assert_eq!(Language::from_tag("pl_PL.UTF-8"), Some(Language::Polish));
        assert_eq!(Language::from_tag("nl-NL"), None);
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
    fn every_language_is_accepted_unless_one_was_asked_for() {
        let app = App::new();
        app.insert(
            0,
            BlockKind::Paragraph,
            "The letter said Straße, mañana and cześć, and one wrod.",
        )
        .unwrap();
        let (choice, speller) = attach(&app, None).unwrap();
        assert_eq!(choice.language, Language::English);
        assert!(choice.every() && speller.is_every());
        assert_eq!(choice.name(), "All languages");
        assert_eq!(app.spelling_language().as_deref(), Some("en-US"));
        let words = |app: &App| -> Vec<String> {
            app.misspellings(0..1).into_iter().map(|w| w.word).collect()
        };
        assert_eq!(words(&app), ["wrod"]);
        // Suggestions are the first language's.
        assert!(app.suggest("wrod").contains(&"word".to_owned()));
        let (choice, speller) = attach(&app, Some(Language::English)).unwrap();
        assert!(!choice.every() && !speller.is_every());
        assert_eq!(choice.name(), "English (US)");
        assert_eq!(words(&app), ["Straße", "mañana", "cześć", "wrod"]);
        let (choice, _) = Setting::In(Language::English).apply(&app).unwrap().unwrap();
        assert!(!choice.every());
        let (choice, _) = Setting::Automatic.apply(&app).unwrap().unwrap();
        assert!(choice.every());
    }

    #[test]
    fn the_personal_list_is_kept_one_word_a_line_and_loses_only_the_line_asked() {
        assert_eq!(personal::word("  Grind "), Ok("Grind".to_owned()));
        assert_eq!(personal::word("Florian’s"), Ok("Florian's".to_owned()));
        assert!(personal::word("").is_err());
        assert!(personal::word("two words").is_err());
        assert!(personal::word("#tag").is_err());
        let text = "# mine\ngrind\nODF\n";
        assert_eq!(personal::without(text, "grind"), "# mine\nODF\n");
        assert_eq!(personal::without(text, "nope"), text);
        let speller = english();
        speller.accept("grnid");
        assert!(speller.knows("grnid"));
        speller.forget("grnid");
        assert!(!speller.knows("grnid"));
    }

    #[test]
    fn each_flag_is_the_dictionarys_own_country() {
        for language in Language::ALL {
            let country = language.tag().split('-').nth(1).unwrap();
            let flag: String = country
                .chars()
                .map(|c| char::from_u32(0x1F1E6 + (c as u32 - 'A' as u32)).unwrap())
                .collect();
            assert_eq!(language.flag(), flag, "{language:?}");
            assert_eq!(language.code(), &language.tag()[..2].to_uppercase());
        }
        assert_eq!(Setting::Automatic.chip(true), "\u{1F310}");
        assert_eq!(Setting::Automatic.chip(false), "ALL");
        assert_eq!(Setting::Off.chip(true), "Off");
        assert_eq!(Setting::In(Language::German).row(false), "DE  Deutsch");
        assert_eq!(Setting::Off.row(true), "Off");
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
    fn the_remembered_setting_is_one_line_per_kind_and_automatic_is_no_line() {
        use grind_core::DocumentKind::{Spreadsheet, Text};
        let text = "# mine\nspreadsheet off\n";
        assert_eq!(preference::parse(text, Spreadsheet), Setting::Off);
        assert_eq!(preference::parse(text, Text), Setting::Automatic);
        assert_eq!(
            preference::parse("text klingon\ntext de\n", Text),
            Setting::In(Language::German)
        );
        let both = preference::with(text, Text, Setting::In(Language::French));
        assert_eq!(both, "# mine\nspreadsheet off\ntext fr-FR\n");
        assert_eq!(
            preference::parse(&both, Text),
            Setting::In(Language::French)
        );
        let back = preference::with(&both, Spreadsheet, Setting::Automatic);
        assert_eq!(back, "# mine\ntext fr-FR\n");
        assert_eq!(preference::with("", Spreadsheet, Setting::Automatic), "");
    }

    #[test]
    fn a_spreadsheet_is_checked_through_the_same_door() {
        let app = grind_sheet::App::new();
        let at = grind_sheet::Pos::new(0, 0);
        app.enter(
            0,
            at,
            "Das ist ein Haus mit einem Fehller.",
            grind_sheet::RecalcMode::Document,
        )
        .unwrap();
        let (choice, _) = attach(&app, None).unwrap();
        assert_eq!(choice.language, Language::German);
        let wrong = app.misspellings(None, None).unwrap();
        assert_eq!(wrong.len(), 1);
        assert_eq!(wrong[0].word, "Fehller");
        assert!(Setting::Off.apply(&app).unwrap().is_none());
        assert!(app.misspellings(None, None).unwrap().is_empty());
    }

    #[test]
    fn a_stated_language_with_no_dictionary_is_not_checked_at_all() {
        let fodt = |lang: &str| {
            format!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
<office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" office:version="1.4" office:mimetype="application/vnd.oasis.opendocument.text">
 <office:styles><style:default-style style:family="paragraph"><style:text-properties fo:language="{lang}"/></style:default-style></office:styles>
 <office:body><office:text><text:p>Hallo wereld</text:p></office:text></office:body>
</office:document>"#
            )
        };
        let app = App::new();
        app.open_bytes("a.fodt", fodt("nl").as_bytes()).unwrap();
        assert_eq!(
            choose(&app, None),
            Err(Unchecked::NoDictionary("nl".into()))
        );
        assert!(attach(&app, None).is_err());
        assert_eq!(app.spelling_language(), None);
        app.open_bytes("a.fodt", fodt("zxx").as_bytes()).unwrap();
        assert_eq!(choose(&app, None), Err(Unchecked::NoLanguage));
        app.open_bytes("a.fodt", fodt("de").as_bytes()).unwrap();
        assert_eq!(choose(&app, None).unwrap().source, Source::Stated);
    }
}
